use std::marker::PhantomData;

use mpmc_bus::Receiver;

/// The condition that caused the [MessageWaiter] to join.
pub enum WaiterJoinCondition {
    /// A ceremony shutdown was initiated.
    Shutdown,
    /// All the messages that the waiter was waiting for have been
    /// received.
    MessagesReceived,
}

impl WaiterJoinCondition {
    pub fn on_messages_received<F>(&self, f: F)
    where
        F: FnOnce(),
    {
        match self {
            WaiterJoinCondition::Shutdown => {}
            WaiterJoinCondition::MessagesReceived => f(),
        }
    }
}

pub enum WaiterClosureResult<S> {
    /// Continue waiting with the updated state.
    Continue(S),
    Join(WaiterJoinCondition),
}

/// See [MessageWaiter::spawn()] or [MessageWaiter::spawn_expected()].
pub struct MessageWaiter<T> {
    join_handle: std::thread::JoinHandle<eyre::Result<WaiterJoinCondition>>,
    message_type: PhantomData<T>,
}

impl<T> MessageWaiter<T>
where
    T: Clone + Sync + Send + 'static,
{
    /// Spawns a thread which received messages via `rx` and passes
    /// them to a `handler` closure which takes some state, and
    /// returns either updated state in
    /// [WaiterClosureResult::Continue] or requests to join the thread
    /// in [WaiterClosureResult::Join]. `initial_state` is the
    /// starting state passed to the `handler` the first time a
    /// message is received.
    pub fn spawn<C, S>(handler: C, initial_state: S, mut rx: Receiver<T>) -> Self
    where
        C: Fn(T, S) -> WaiterClosureResult<S> + Send + 'static,
        S: Send + 'static,
    {
        let join_handle = std::thread::spawn(move || {
            let mut state = initial_state;

            loop {
                let received_message = rx.recv()?;

                match handler(received_message, state) {
                    WaiterClosureResult::Continue(new_state) => {
                        state = new_state;
                        continue;
                    }
                    WaiterClosureResult::Join(join) => {
                        return Ok(join);
                    }
                }
            }
        });

        Self {
            join_handle,
            message_type: PhantomData,
        }
    }

    /// Wait for all the expected messages to be received.
    pub fn join(self) -> eyre::Result<WaiterJoinCondition> {
        match self.join_handle.join() {
            Err(_panic_error) => panic!("Thread panicked"),
            Ok(Err(run_error)) => Err(run_error),
            Ok(Ok(join_condition)) => Ok(join_condition),
        }
    }
}

pub trait IsShutdownMessage {
    /// Returns `true` if this message a shutdown message, otherwise
    /// returns `false`.
    fn is_shutdown_message(&self) -> bool;
}

impl<T> MessageWaiter<T>
where
    T: IsShutdownMessage + PartialEq + Clone + Sync + Send + 'static,
{
    /// Spawns a thread that listens to `rx` until all messages in
    /// `expected_messages` have been received once, or if the
    /// shutdown message specified by the [IsShutdownMessage] trait is
    /// received. Call [MessageWaiter::join()] to block until all
    /// expected messages have been received.
    pub fn spawn_expected(expected_messages: Vec<T>, rx: Receiver<T>) -> Self {
        Self::spawn(
            move |message, state| Self::vec_waiter(message, state),
            expected_messages,
            rx,
        )
    }

    /// `handler` closure used in [MessageWaiter::spawn_expected()].
    fn vec_waiter(
        received_message: T,
        mut expected_messages: Vec<T>,
    ) -> WaiterClosureResult<Vec<T>> {
        if received_message.is_shutdown_message() {
            return WaiterClosureResult::Join(WaiterJoinCondition::Shutdown);
        }

        if let Some(position) = expected_messages
            .iter()
            .position(|message| message == &received_message)
        {
            expected_messages.swap_remove(position);
        }

        if expected_messages.is_empty() {
            WaiterClosureResult::Join(WaiterJoinCondition::MessagesReceived)
        } else {
            WaiterClosureResult::Continue(expected_messages)
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::{Arc, Mutex};

    use crate::waiter::IsShutdownMessage;

    use super::MessageWaiter;
    use mpmc_bus::Bus;

    #[test]
    fn test_spawn_expected() {
        let bus = Bus::<u8>::new(100);
        let rx = bus.subscribe();

        impl IsShutdownMessage for u8 {
            fn is_shutdown_message(&self) -> bool {
                *self == 0
            }
        }

        let waiter = MessageWaiter::spawn_expected(vec![1, 2, 3], rx);

        let has_joined = Arc::new(Mutex::new(false));

        let has_joined_thread = has_joined.clone();
        let waiter_joiner = std::thread::spawn(move || {
            waiter.join().unwrap();
            *has_joined_thread.lock().unwrap() = true;
        });

        assert!(!*has_joined.lock().unwrap());
        bus.broadcast(1).unwrap();
        assert!(!*has_joined.lock().unwrap());
        bus.broadcast(2).unwrap();
        assert!(!*has_joined.lock().unwrap());
        bus.broadcast(3).unwrap();

        waiter_joiner.join().unwrap();
        assert!(*has_joined.lock().unwrap());
    }
}

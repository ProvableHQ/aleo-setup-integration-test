use std::marker::PhantomData;

use mpmc_bus::Receiver;

use crate::join::MultiJoinable;

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

impl<T> std::fmt::Debug for MessageWaiter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageWaiter")
            .field("join_handle", &self.join_handle)
            .finish()
    }
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
    pub fn spawn<H, S, J>(
        handler: H,
        on_messages_received: J,
        initial_state: S,
        mut rx: Receiver<T>,
    ) -> Self
    where
        H: Fn(T, S) -> WaiterClosureResult<S> + Send + 'static,
        S: Send + 'static,
        J: FnOnce() -> eyre::Result<()> + Send + 'static,
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
                        match &join {
                            WaiterJoinCondition::Shutdown => {}
                            WaiterJoinCondition::MessagesReceived => on_messages_received()?,
                        }

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

impl<T> MultiJoinable for MessageWaiter<T>
where
    T: Clone + Sync + Send + 'static,
{
    fn join(self: Box<Self>) -> std::thread::Result<()> {
        match MessageWaiter::join(*self) {
            Ok(_join_condition) => Ok(()),
            Err(error) => Err(Box::new(error)),
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
    pub fn spawn_expected<J>(
        expected_messages: Vec<T>,
        on_messages_received: J,
        rx: Receiver<T>,
    ) -> Self
    where
        J: FnOnce() -> eyre::Result<()> + Send + 'static,
    {
        Self::spawn(
            Self::vec_waiter,
            on_messages_received,
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
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    use crate::waiter::IsShutdownMessage;

    use super::MessageWaiter;
    use mpmc_bus::Bus;

    impl IsShutdownMessage for u8 {
        fn is_shutdown_message(&self) -> bool {
            *self == 0
        }
    }

    #[test]
    fn test_spawn_expected() {
        let bus = Bus::<u8>::new(100);
        let rx = bus.subscribe();

        let on_messages_received = AtomicBool::new(false);
        let waiter = MessageWaiter::spawn_expected(
            vec![1, 2, 3],
            move || {
                on_messages_received.store(true, Ordering::SeqCst);
                Ok(())
            },
            rx,
        );

        let has_joined = AtomicBool::new(false);
        let has_joined_ref = &has_joined;
        let waiter_joiner = std::thread::spawn(move || {
            waiter.join().unwrap();
            has_joined.store(true, Ordering::SeqCst);
        });

        assert!(!on_messages_received.load(Ordering::SeqCst));

        assert!(!has_joined.load(Ordering::SeqCst));
        bus.broadcast(1).unwrap();
        assert!(!has_joined.load(Ordering::SeqCst));
        bus.broadcast(2).unwrap();
        assert!(!has_joined.load(Ordering::SeqCst));
        bus.broadcast(3).unwrap();

        waiter_joiner.join().unwrap();
        assert!(has_joined.load(Ordering::SeqCst));
        assert!(on_messages_received.load(Ordering::SeqCst));
    }

    /// Test that upon shutdown the waiter is joined, but the
    /// on_messages_receieved closure is not invoked.
    #[test]
    fn test_spawn_expected_shutdown() {
        let bus = Bus::<u8>::new(100);
        let rx = bus.subscribe();

        let on_messages_received = Arc::new(Mutex::new(false));
        let on_messages_received_thread = on_messages_received.clone();
        let waiter = MessageWaiter::spawn_expected(
            vec![1, 2, 3],
            move || {
                *on_messages_received_thread.lock().unwrap() = true;
                Ok(())
            },
            rx,
        );

        let has_joined = Arc::new(Mutex::new(false));

        let has_joined_thread = has_joined.clone();
        let waiter_joiner = std::thread::spawn(move || {
            waiter.join().unwrap();
            *has_joined_thread.lock().unwrap() = true;
        });

        assert!(!*on_messages_received.lock().unwrap());

        assert!(!*has_joined.lock().unwrap());
        bus.broadcast(1).unwrap();
        assert!(!*has_joined.lock().unwrap());
        bus.broadcast(0).unwrap();

        waiter_joiner.join().unwrap();

        assert!(*has_joined.lock().unwrap());
        assert!(!*on_messages_received.lock().unwrap());
    }
}

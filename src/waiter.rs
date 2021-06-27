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

/// See [MessageWaiter::spawn()].
pub struct MessageWaiter<T> {
    join_handle: std::thread::JoinHandle<eyre::Result<WaiterJoinCondition>>,
    message_type: PhantomData<T>,
}

impl<T> MessageWaiter<T>
where
    T: PartialEq + Clone + Sync + Send + 'static,
{
    /// Spawns a thread that listens to `rx` until all messages in
    /// `expected_messages` have been received once, or if the
    /// specified `shutdown_message` is received. Call
    /// [MessageWaiter::join()] to block until all expected messages
    /// have been received.
    pub fn spawn<S>(expected_messages: Vec<T>, is_shutdown_message: S, rx: Receiver<T>) -> Self
    where
        S: Fn(&T) -> bool + Send + 'static,
    {
        let join_handle =
            std::thread::spawn(move || Self::listen(expected_messages, is_shutdown_message, rx));

        Self {
            join_handle,
            message_type: PhantomData,
        }
    }

    /// Listen to messages from `rx`, and remove equivalent message
    /// from `expected_messages` until `expected_messages` is empty.
    fn listen<S>(
        mut expected_messages: Vec<T>,
        is_shutdown_message: S,
        mut rx: Receiver<T>,
    ) -> eyre::Result<WaiterJoinCondition>
    where
        S: Fn(&T) -> bool,
    {
        while !expected_messages.is_empty() {
            let received_message = rx.recv()?;

            if is_shutdown_message(&received_message) {
                return Ok(WaiterJoinCondition::Shutdown);
            }

            if let Some(position) = expected_messages
                .iter()
                .position(|message| message == &received_message)
            {
                expected_messages.swap_remove(position);
            }
        }

        Ok(WaiterJoinCondition::MessagesReceived)
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

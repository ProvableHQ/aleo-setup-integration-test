use std::{fmt::Display, marker::PhantomData};

use mpmc_bus::Receiver;

pub mod contributor;
pub mod coordinator;
pub mod coordinator_proxy;
pub mod npm;
pub mod process;
pub mod rust;
pub mod verifier;

/// Message sent between the various components running during the
/// setup ceremony. Each component will have a process monitor running
/// in its own thread which will listen to these messages.
#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub enum CeremonyMessage {
    /// Notify the receivers that the specified round has finished
    /// sucessfully.
    RoundFinished(u64),
    /// Notify the receivers that the specified round has successfully
    /// been aggregated.
    RoundAggregated(u64),
    /// Notify the receivers that the coordinator rocket server is
    /// ready to start receiving requests.
    CoordinatorReady,
    /// Notify the receivers that the cordinator nodejs proxy is ready
    /// to start receiving requests.
    CoordinatorProxyReady,
    /// Tell all the recievers to shut down.
    Shutdown,
}

/// Which phase of the setup is to be run.
///
/// TODO: confirm is "Phase" the correct terminology here?
#[derive(Debug, Clone, Copy)]
pub enum SetupPhase {
    Development,
    Inner,
    Outer,
    Universal,
}

impl Display for SetupPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SetupPhase::Development => "development",
            SetupPhase::Inner => "inner",
            SetupPhase::Outer => "outer",
            SetupPhase::Universal => "universal",
        };

        write!(f, "{}", s)
    }
}

/// See [MessageWaiter::spawn()].
pub struct MessageWaiter<T> {
    join_handle: std::thread::JoinHandle<eyre::Result<()>>,
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
    pub fn spawn(expected_messages: Vec<T>, shutdown_message: T, rx: Receiver<T>) -> Self {
        let join_handle =
            std::thread::spawn(move || Self::listen(expected_messages, shutdown_message, rx));

        Self {
            join_handle,
            message_type: PhantomData,
        }
    }

    /// Listen to messages from `rx`, and remove equivalent message
    /// from `expected_messages` until `expected_messages` is empty.
    fn listen(
        mut expected_messages: Vec<T>,
        shutdown_message: T,
        mut rx: Receiver<T>,
    ) -> eyre::Result<()> {
        while !expected_messages.is_empty() {
            let received_message = rx.recv()?;

            if received_message == shutdown_message {
                break;
            }

            if let Some(position) = expected_messages
                .iter()
                .position(|message| message == &received_message)
            {
                expected_messages.swap_remove(position);
            }
        }

        Ok(())
    }

    /// Wait for all the expected messages to be received.
    pub fn join(self) -> eyre::Result<()> {
        match self.join_handle.join() {
            Err(panic_error) => panic!(panic_error),
            Ok(Err(run_error)) => Err(run_error),
            Ok(Ok(_)) => Ok(()),
        }
    }
}

use std::{any::Any, fmt::Display, marker::PhantomData};

use mpmc_bus::{Receiver, RecvError};
use thiserror::Error;

pub mod contributor;
pub mod coordinator;
pub mod coordinator_proxy;
pub mod npm;
pub mod process;
pub mod rust;

/// Message sent between the various components running during the
/// setup ceremony. Each component will have a process monitor running
/// in its own thread which will listen to these messages.
#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub enum CeremonyMessage {
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

#[derive(Debug)]
pub enum MessageWaiterError {
    Recv(RecvError),
    Panic(Box<(dyn Any + Send + 'static)>),
}

impl std::fmt::Display for MessageWaiterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageWaiterError::Recv(recv_error) => {
                write!(f, "Error while receiving message from bus: {}", recv_error)
            }
            MessageWaiterError::Panic(_) => write!(f, "Panic in MessageWaiter thread"),
        }
    }
}

impl std::error::Error for MessageWaiterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MessageWaiterError::Recv(error) => Some(error),
            MessageWaiterError::Panic(_) => None,
        }
    }
}

pub struct MessageWaiter<T> {
    join_handle: std::thread::JoinHandle<Result<(), RecvError>>,
    message_type: PhantomData<T>,
}

impl<T> MessageWaiter<T>
where
    T: PartialEq + Clone + Sync + Send + 'static,
{
    pub fn new(messages: Vec<T>, rx: Receiver<T>, shutdown_message: T) -> Self {
        let join_handle = std::thread::spawn(move || Self::listen(messages, rx, shutdown_message));

        Self {
            join_handle,
            message_type: PhantomData,
        }
    }

    fn listen(
        mut messages: Vec<T>,
        mut rx: Receiver<T>,
        shutdown_message: T,
    ) -> Result<(), RecvError> {
        while !messages.is_empty() {
            let received_message = rx.recv()?;

            if received_message == shutdown_message {
                break;
            }

            if let Some(position) = messages
                .iter()
                .position(|message| message == &received_message)
            {
                messages.swap_remove(position);
            }
        }

        Ok(())
    }

    pub fn join(mut self) -> Result<(), MessageWaiterError> {
        match self.join_handle.join() {
            Err(panic_error) => Err(MessageWaiterError::Panic(panic_error)),
            Ok(Err(recv_error)) => Err(MessageWaiterError::Recv(recv_error)),
            Ok(Ok(_)) => Ok(()),
        }
    }
}

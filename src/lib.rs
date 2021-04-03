use mpmc_bus::Receiver;
use serde::{Deserialize, Serialize};

use std::{fmt::Display, marker::PhantomData, str::FromStr};

pub mod contributor;
pub mod coordinator;
pub mod coordinator_proxy;
pub mod drop_participant;
pub mod git;
pub mod multi;
pub mod npm;
pub mod options;
pub mod process;
pub mod reporting;
pub mod rust;
pub mod state_monitor;
pub mod test;
pub mod time_limit;
pub mod util;
pub mod verifier;

/// Type of participant in the ceremony.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParticipantType {
    Contributor,
    Verifier,
}

impl FromStr for ParticipantType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contributor" => Ok(Self::Contributor),
            "verifier" => Ok(Self::Verifier),
            _ => Err(eyre::eyre!(
                "Unable to parse ParticipantType from str: {:?}",
                s
            )),
        }
    }
}

/// A reference to a participant in the ceremony.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Participant {
    pub participant_type: ParticipantType,
    /// Public aleo address e.g.
    /// `aleo18whcjapew3smcwnj9lzk29vdhpthzank269vd2ne24k0l9dduqpqfjqlda`
    pub address: String,
}

/// Message sent between the various components running during the
/// setup ceremony. Each component will have a process monitor running
/// in its own thread which will listen to these messages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CeremonyMessage {
    /// Notify the receivers that the specified round has started.
    RoundStarted(u64),
    /// Notify the receivers that the specified round has completed
    /// verification, and aggregation of the contributions by the
    /// coordinator has begun.
    RoundStartedAggregation(u64),
    /// Notify the receivers that the specified round has successfully
    /// been aggregated.
    RoundAggregated(u64),
    /// Notify the receivers that the specified round has finished
    /// sucessfully.
    RoundFinished(u64),
    /// Notify the receivers that the coordinator is ready and waiting
    /// for participants for the specified round before starting it.
    RoundWaitingForParticipants(u64),
    /// Notify the receivers that the cordinator nodejs proxy is ready
    /// to start receiving requests.
    CoordinatorProxyReady,
    /// Notify the receivers that the coordinator has just dropped a
    /// participant in the current round.
    ParticipantDropped(Participant),
    /// Tell all the recievers to shut down.
    Shutdown,
}

/// Which phase of the setup is to be run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Environment {
    #[serde(rename = "development")]
    Development,
    #[serde(rename = "inner")]
    Inner,
    #[serde(rename = "outer")]
    Outer,
    #[serde(rename = "universal")]
    Universal,
}

impl Environment {
    /// Available variants that can be parsed with [FromStr].
    pub fn str_variants() -> &'static [&'static str] {
        &["development", "inner", "outer", "universal"]
    }
}

impl FromStr for Environment {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "development" => Ok(Self::Development),
            "inner" => Ok(Self::Inner),
            "outer" => Ok(Self::Outer),
            "universal" => Ok(Self::Universal),
            _ => Err(eyre::eyre!("unable to parse {:?} as a SetupPhase", s)),
        }
    }
}

impl Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Environment::Development => "development",
            Environment::Inner => "inner",
            Environment::Outer => "outer",
            Environment::Universal => "universal",
        };

        write!(f, "{}", s)
    }
}

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
    ) -> eyre::Result<WaiterJoinCondition> {
        while !expected_messages.is_empty() {
            let received_message = rx.recv()?;

            if received_message == shutdown_message {
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

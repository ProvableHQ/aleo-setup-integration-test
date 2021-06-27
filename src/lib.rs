use mpmc_bus::Receiver;
use serde::{Deserialize, Serialize};

use std::{fmt::Display, marker::PhantomData, str::FromStr};

pub mod contributor;
pub mod coordinator;
pub mod drop_participant;
pub mod git;
pub mod npm;
pub mod options;
pub mod process;
pub mod reporting;
pub mod rust;
pub mod specification;
pub mod state_monitor;
pub mod test;
pub mod time_limit;
pub mod util;
pub mod verifier;

/// A reference to a contributor in the ceremony.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ContributorRef {
    /// Public aleo address
    pub address: AleoPublicKey,
}

impl std::fmt::Display for ContributorRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.address.fmt(f)
    }
}

/// A reference to a verifier in the ceremony.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct VerifierRef {
    /// Public aleo address
    pub address: AleoPublicKey,
}

/// A reference to a participant in the ceremony.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ParticipantRef {
    Contributor(ContributorRef),
    Verifier(VerifierRef),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownReason {
    Error,
    TestFinished,
}

impl std::fmt::Display for ShutdownReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShutdownReason::Error => f.write_str("there was an error"),
            ShutdownReason::TestFinished => todo!("the test is finished"),
        }
    }
}

/// Message sent between the various components running during the
/// setup ceremony. Each component will have a process monitor running
/// in its own thread which will listen to these messages.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CeremonyMessage {
    /// Notify the receivers that the specified round has started.
    /// Data is the round number.
    RoundStarted(u64),
    /// Notify the receivers that the specified round has completed
    /// verification, and aggregation of the contributions by the
    /// coordinator has begun.
    /// Data is the round number.
    RoundStartedAggregation(u64),
    /// Notify the receivers that the specified round has successfully
    /// been aggregated.
    /// Data is the round number.
    RoundAggregated(u64),
    /// Notify the receivers that the specified round has finished
    /// sucessfully.
    /// Data is the round number.
    RoundFinished(u64),
    /// Notify the receivers that the coordinator is ready and waiting
    /// for participants for the specified round before starting it.
    /// Data is the round number.
    RoundWaitingForParticipants(u64),
    /// Notify the receivers that the coordinator has just dropped a
    /// participant in the current round.
    ParticipantDropped(ParticipantRef),
    /// The coordinator has successfully received a contribution from
    /// a contributor at a given chunk.
    SuccessfulContribution {
        contributor: ContributorRef,
        chunk: u64,
    },
    /// Tell all the recievers to shut down.
    Shutdown(ShutdownReason),
}

impl CeremonyMessage {
    pub fn is_shutdown(&self) -> bool {
        match self {
            Self::Shutdown(_) => true,
            _ => false,
        }
    }
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

impl Default for Environment {
    fn default() -> Self {
        Self::Development
    }
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

/// An aleo public key e.g.
/// `aleo1hsr8czcmxxanpv6cvwct75wep5ldhd2s702zm8la47dwcxjveypqsv7689`
///
/// TODO: implement deserialize myself to include the FromStr
/// implementation's validation.
#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AleoPublicKey(String);

impl std::str::FromStr for AleoPublicKey {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 63 {
            return Err(eyre::eyre!(
                "String is not the required 63 characters in length: {:?}",
                s
            ));
        }

        let (key_type, _key) = s.split_at(4);
        if key_type != "aleo" {
            return Err(eyre::eyre!("Key is not an `aleo` type key {:?}", s));
        }

        Ok(AleoPublicKey(s.to_string()))
    }
}

impl Display for AleoPublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for AleoPublicKey {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

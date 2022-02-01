use serde::{Deserialize, Serialize};
use waiter::IsShutdownMessage;

use std::{fmt::Display, str::FromStr};

pub mod browser_contributor;
pub mod ceremony_waiter;
pub mod cli_contributor;
pub mod config;
pub mod coordinator;
pub mod drop_participant;
pub mod frontend;
pub mod git;
pub mod join;
pub mod npm;
pub mod options;
pub mod process;
pub mod reporting;
pub mod run;
pub mod rust;
pub mod specification;
pub mod state_monitor;
pub mod test;
pub mod time_limit;
pub mod util;
pub mod verifier;
pub mod waiter;

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

impl IsShutdownMessage for CeremonyMessage {
    fn is_shutdown_message(&self) -> bool {
        matches!(self, Self::Shutdown(_))
    }
}

/// Messages pertinent to the running of the entire integration test specification.
pub enum IntegrationTestMessage {
    /// The frontend npm server has started.
    /// 
    FrontendStarted
}

impl IsShutdownMessage for IntegrationTestMessage {
    fn is_shutdown_message(&self) -> bool {
        false
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

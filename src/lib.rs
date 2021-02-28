use std::fmt::Display;

pub mod coordinator;
pub mod coordinator_proxy;
pub mod npm;
pub mod process;
pub mod rust;

/// Message sent between the various components running during the
/// setup ceremony. Each component will have a process monitor running
/// in its own thread which will listen to these messages.
#[derive(Clone, Debug, Copy)]
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

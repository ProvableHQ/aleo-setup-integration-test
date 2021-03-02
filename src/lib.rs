use std::fmt::Display;

use mpmc_bus::Receiver;

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

/// Wait for the specified `messages` to arrive in the `ceremony_rx` bus receiver.
pub fn wait_for_messages(mut ceremony_rx: Receiver<CeremonyMessage>, message: CeremonyMessage) -> eyre::Result<()> {
    for message in ceremony_rx.recv() {
        match message {
            CeremonyMessage::Shutdown => {
                return Err(eyre::eyre!(
                    "Ceremony shutdown before coordinator could start."
                ))
            }
            msg if msg == message => {
                break;
            } 
            _ => {
                tracing::error!("Unexpected message: {:?}", message);
            }
        }
    }

    Ok(())
}
use std::path::PathBuf;

use flume::{Receiver, Sender};
use subprocess::Exec;

use crate::{process::default_parse_exit_status, CeremonyMessage, SetupPhase};

/// Configuration for the [run_coordinator()] function to run
/// `aleo-setup-coordinator` rocket server.
#[derive(Debug)]
pub struct CoordinatorConfig {
    /// The location of the `aleo-setup-coordinator` repository.
    pub crate_dir: PathBuf,
    /// The location of the `aleo-setup-coordinator` binary (including
    /// the binary name).
    pub setup_coordinator_bin: PathBuf,
    /// What phase of the setup ceremony to run.
    pub phase: SetupPhase,
}

/// Run the `aleo-setup-coordinator`. This will first wait for the
/// nodejs proxy to start (which will publish a
/// [CoordinatorMessage::CoordinatorProxyReady]).
///
/// TODO: return a thread join handle.
/// TODO: make a monitor thread (like in the proxy).
pub fn run_coordinator(
    config: CoordinatorConfig,
    coordinator_tx: Sender<CeremonyMessage>,
    coordinator_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<()> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Setup coordinator waiting for nodejs proxy to start.");

    // Wait for the coordinator proxy to report that it's ready.
    for message in coordinator_rx.recv() {
        match message {
            CeremonyMessage::CoordinatorProxyReady => break,
            CeremonyMessage::Shutdown => return Ok(()),
            _ => {
                tracing::error!("Unexpected message: {:?}", message);
            }
        }
    }

    tracing::info!("Starting setup coordinator.");

    Exec::cmd(std::fs::canonicalize(config.setup_coordinator_bin)?)
        .cwd(config.crate_dir)
        .arg(config.phase.to_string())
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)?;

    // TODO: wait for the `Rocket has launched from` message on
    // STDOUT, just like how it is implemented in
    // run_coordinator_proxy(), then send the
    // `CoordinatorMessage::CoordinatorReady` to notify the verifier
    // and the participants that they can start.

    Ok(())
}

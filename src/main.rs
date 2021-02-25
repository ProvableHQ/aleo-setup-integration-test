//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use aleo_setup_integration_test::{CoordinatorMessage, SetupPhase, coordinator_proxy::run_coordinator_proxy, npm::npm_install, process::parse_exit_status, rust::{RustToolchain, build_rust_crate, install_rust_toolchain}};
use flume::{Receiver, Sender};
use subprocess::Exec;
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

use std::{
    fmt::Debug,
    path::PathBuf,
    str::FromStr,
};

/// Set up [tracing] and [color-eyre](color_eyre).
fn setup_reporting() -> eyre::Result<()> {
    color_eyre::install()?;

    let fmt_layer = tracing_subscriber::fmt::layer();
    let error_layer = tracing_error::ErrorLayer::default();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(error_layer)
        .init();

    Ok(())
}

/// Configuration for the [run_coordinator()] function to run
/// `aleo-setup-coordinator` rocket server.
#[derive(Debug)]
struct CoordinatorConfig {
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
fn run_coordinator(
    config: CoordinatorConfig,
    coordinator_tx: Sender<CoordinatorMessage>,
    coordinator_rx: Receiver<CoordinatorMessage>,
) -> eyre::Result<()> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Setup coordinator waiting for nodejs proxy to start.");

    // Wait for the coordinator proxy to report that it's ready.
    for message in coordinator_rx.recv() {
        match message {
            CoordinatorMessage::CoordinatorProxyReady => break,
            CoordinatorMessage::Shutdown => return Ok(()),
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
        .and_then(parse_exit_status)?;

    // TODO: wait for the `Rocket has launched from` message on
    // STDOUT, just like how it is implemented in
    // run_coordinator_proxy(), then send the
    // `CoordinatorMessage::CoordinatorReady` to notify the verifier
    // and the participants that they can start.

    Ok(())
}

/// The directory that the `aleo-setup-coordinator` repository is
/// cloned to.
const SETUP_COORDINATOR_DIR: &str = "aleo-setup-coordinator";

/// The directory that the `aleo-setup` repository is cloned to.
const SETUP_DIR: &str = "aleo-setup";

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
fn main() -> eyre::Result<()> {
    setup_reporting()?;

    // Install a specific version of the rust toolchain needed to be
    // able to compile `aleo-setup`.
    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());
    install_rust_toolchain(&rust_1_47_nightly)?;

    // Clone the git repos for `aleo-setup` and
    // `aleo-setup-coordinator`.
    //
    // **NOTE: currently I am commenting out these lines during
    // development of this test**
    //
    // TODO: implement a command line argument that will ignore this
    // step if the repos are already cloned, for development purposes.
    // In the actual test it's probably good for this to fail if it's
    // trying to overwrite a previous test, it should be starting
    // clean.
    // get_git_repository(
    //     "https://github.com/AleoHQ/aleo-setup-coordinator",
    //     SETUP_COORDINATOR_DIR,
    // )?;
    // get_git_repository("https://github.com/AleoHQ/aleo-setup", SETUP_DIR)?;

    // Build the setup coordinator Rust project.
    let coordinator_output_dir = build_rust_crate(SETUP_COORDINATOR_DIR, &rust_1_47_nightly)?;
    let coordinator_bin = coordinator_output_dir.join("aleo-setup-coordinator");

    // Install the dependencies for the setup coordinator nodejs proxy.
    npm_install(SETUP_COORDINATOR_DIR)?;

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let (coordinator_tx, coordinator_rx) = flume::unbounded::<CoordinatorMessage>();

    // Run the nodejs proxy server for the coordinator.
    let coordinator_proxy_join = run_coordinator_proxy(
        SETUP_COORDINATOR_DIR,
        coordinator_tx.clone(),
        coordinator_rx.clone(),
    )?;

    let coordinator_config = CoordinatorConfig {
        crate_dir: PathBuf::from_str(SETUP_COORDINATOR_DIR)?,
        setup_coordinator_bin: coordinator_bin,
        phase: SetupPhase::Development,
    };

    // Run the coordinator (which will first wait for the proxy to start).
    run_coordinator(
        coordinator_config,
        coordinator_tx.clone(),
        coordinator_rx.clone(),
    )?;

    // TODO: start the `setup1-verifier` and `setup1-contributor`.

    tracing::debug!("Test complete, waiting for the other threads to shutdown safely");

    // Tell the other threads to shutdown, safely terminating their
    // child processes.
    coordinator_tx
        .send(CoordinatorMessage::Shutdown)
        .expect("unable to send message");

    // Wait for the setup proxy threads to close after being told to shut down.
    coordinator_proxy_join
        .join()
        .expect("error while joining setup proxy threads");

    Ok(())
}

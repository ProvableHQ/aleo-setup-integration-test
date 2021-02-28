//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use aleo_setup_integration_test::{
    coordinator::{run_coordinator, CoordinatorConfig},
    coordinator_proxy::run_coordinator_proxy,
    npm::npm_install,
    rust::{build_rust_crate, install_rust_toolchain, RustToolchain},
    CeremonyMessage, SetupPhase,
};
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

use std::{path::PathBuf, str::FromStr};

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
    let (coordinator_tx, coordinator_rx) = flume::unbounded::<CeremonyMessage>();

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
        .send(CeremonyMessage::Shutdown)
        .expect("unable to send message");

    // Wait for the setup proxy threads to close after being told to shut down.
    coordinator_proxy_join
        .join()
        .expect("error while joining setup proxy threads");

    Ok(())
}

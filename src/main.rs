//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use aleo_setup_integration_test::{
    contributor::{generate_contributor_key, run_contributor},
    coordinator::{run_coordinator, CoordinatorConfig},
    coordinator_proxy::run_coordinator_proxy,
    npm::npm_install,
    rust::{build_rust_crate, install_rust_toolchain, RustToolchain},
    verifier::run_verifier,
    CeremonyMessage, MessageWaiter, SetupPhase,
};
use eyre::Context;
use mpmc_bus::Bus;
use tracing_subscriber::{
    prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

/// Set up [tracing] and [color-eyre](color_eyre).
fn setup_reporting() -> eyre::Result<()> {
    color_eyre::install()?;

    let filter_layer = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("info"))?;
    let fmt_layer = tracing_subscriber::fmt::layer();
    let error_layer = tracing_error::ErrorLayer::default();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(error_layer)
        .init();

    Ok(())
}

/// The directory that the `aleo-setup-coordinator` repository is
/// cloned to.
const COORDINATOR_DIR: &str = "aleo-setup-coordinator";

/// The directory that the `aleo-setup` repository is cloned to.
const SETUP_DIR: &str = "aleo-setup";

/// URL used by the contributors and verifiers to connect to the
/// coordinator.
const COORDINATOR_API_URL: &str = "http://localhost:9000";

/// Path to contributor 1's key file.
const CONTRIBUTOR1_KEY_PATH: &str = "contributor1_key.json";

/// Path to where the log files are stored.
const LOG_DIR: &str = "logs";

/// Path to where the key files are stored.
const KEYS_DIR: &str = "keys";

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
fn main() -> eyre::Result<()> {
    setup_reporting()?;

    let setup_phase = SetupPhase::Development;

    // Create the log dir if it doesn't yet exist.
    let log_dir_path = Path::new(LOG_DIR);
    if !log_dir_path.exists() {
        std::fs::create_dir(log_dir_path)?;
    }

    // Create the keys dir if it doesn't yet exist.
    let keys_dir_path = Path::new(KEYS_DIR);
    if !keys_dir_path.exists() {
        std::fs::create_dir(keys_dir_path)?;
    }

    // Install a specific version of the rust toolchain needed to be
    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());
    // able to compile `aleo-setup`.
    // install_rust_toolchain(&rust_1_47_nightly)?;

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
    build_rust_crate(COORDINATOR_DIR, &rust_1_47_nightly)?;
    let coordinator_bin_path = Path::new(COORDINATOR_DIR)
        .join("target/release")
        .join("aleo-setup-coordinator");

    // Install the dependencies for the setup coordinator nodejs proxy.
    // npm_install(SETUP_COORDINATOR_DIR)?;

    // Build the setup1-contributor Rust project.
    build_rust_crate(
        Path::new(SETUP_DIR).join("setup1-contributor"),
        &rust_1_47_nightly,
    )?;

    // Build the setup1-verifier Rust project.
    build_rust_crate(
        Path::new(SETUP_DIR).join("setup1-verifier"),
        &rust_1_47_nightly,
    )?;

    // Output directory for setup1-verifier and setup1-contributor
    // projects.
    let setup_output_dir = Path::new(SETUP_DIR).join("target/release");

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let bus: Bus<CeremonyMessage> = Bus::new(1000);
    let ceremony_tx = bus.broadcaster();
    let ceremony_rx = bus.subscribe();

    let contributor_bin_path = setup_output_dir.join("aleo-setup-contributor");
    let contributor1_key_file_path = keys_dir_path.join("contributor1-key.json");
    generate_contributor_key(&contributor_bin_path, &contributor1_key_file_path)?;

    // Watches the bus to determine when the coordinator and coordinator proxy are ready.
    let coordinator_ready = MessageWaiter::spawn(
        vec![
            CeremonyMessage::CoordinatorReady,
            CeremonyMessage::CoordinatorProxyReady,
        ],
        CeremonyMessage::Shutdown,
        ceremony_rx.clone(),
    );

    let round1_finished = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundFinished(1)],
        CeremonyMessage::Shutdown,
        ceremony_rx.clone(),
    );

    let round1_aggregated = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundAggregated(1)],
        CeremonyMessage::Shutdown,
        ceremony_rx.clone(),
    );

    // Run the nodejs proxy server for the coordinator.
    let coordinator_proxy_join = run_coordinator_proxy(
        COORDINATOR_DIR,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        log_dir_path.to_path_buf(),
    )?;

    let coordinator_config = CoordinatorConfig {
        crate_dir: PathBuf::from_str(COORDINATOR_DIR)?,
        setup_coordinator_bin: coordinator_bin_path,
        phase: setup_phase,
    };

    // Run the coordinator.
    let coordinator_join = run_coordinator(
        coordinator_config,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        log_dir_path.to_path_buf(),
    )?;

    // Wait for the coordinator and coordinator proxy to start.
    coordinator_ready
        .join()
        .wrap_err("Error while waiting for coordinator to start")?;

    tracing::info!("Coordinator started.");

    let contributor_join = run_contributor(
        contributor_bin_path,
        &contributor1_key_file_path,
        setup_phase,
        COORDINATOR_API_URL.to_string(),
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        log_dir_path.to_path_buf(),
    )?;

    let verifier_bin_path = setup_output_dir.join("setup1-verifier");
    let verifier_join = run_verifier(
        verifier_bin_path,
        setup_phase,
        COORDINATOR_API_URL.to_string(),
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        log_dir_path.to_path_buf(),
    )?;

    round1_finished
        .join()
        .wrap_err("Error while waiting for round 1 to finish")?;

    tracing::info!("Round 1 Finished (waiting for aggregation to complete).");

    round1_aggregated
        .join()
        .wrap_err("Error while waiting for round 1 to aggregate")?;

    tracing::info!("Round 1 Aggregated, test complete.");

    // Tell the other threads to shutdown, safely terminating their
    // child processes.
    ceremony_tx
        .broadcast(CeremonyMessage::Shutdown)
        .expect("unable to send message");

    // Wait for contributor threads to close after being told to shut down.
    contributor_join
        .join()
        .expect("error while joining contributor threads");

    // Wait for verifier threads to close after being told to shut down.
    verifier_join
        .join()
        .expect("error while joining verifier threads");

    // Wait for the coordinator threads to close after being told to shut down.
    coordinator_join
        .join()
        .expect("error while joining coordinator threads");

    // Wait for the coordinator proxy threads to close after being told to shut down.
    coordinator_proxy_join
        .join()
        .expect("error while joining coordinator proxy threads");

    Ok(())
}

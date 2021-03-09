//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use aleo_setup_integration_test::{
    contributor::{generate_contributor_key, run_contributor},
    coordinator::{deploy_coordinator_rocket_config, run_coordinator, CoordinatorConfig},
    coordinator_proxy::run_coordinator_proxy,
    git::clone_git_repository,
    npm::npm_install,
    reporting::setup_reporting,
    rust::{build_rust_crate, install_rust_toolchain, RustToolchain},
    verifier::run_verifier,
    CeremonyMessage, MessageWaiter, SetupPhase,
};
use eyre::Context;
use mpmc_bus::Bus;
use structopt::StructOpt;

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

/// The url for the `aleo-setup-coordinator` git repository.
const COORDINATOR_REPO_URL: &str = "https://github.com/AleoHQ/aleo-setup-coordinator";

/// The directory that the `aleo-setup-coordinator` repository is
/// cloned to.
const COORDINATOR_DIR: &str = "aleo-setup-coordinator";

/// The url for the `aleo-setup` git repository.
const SETUP_REPO_URL: &str = "https://github.com/AleoHQ/aleo-setup";

/// The directory that the `aleo-setup` repository is cloned to.
const SETUP_DIR: &str = "aleo-setup";

/// URL used by the contributors and verifiers to connect to the
/// coordinator.
const COORDINATOR_API_URL: &str = "http://localhost:9000";

/// Path to where the log files, key files and transcripts are stored.
const OUT_DIR: &str = "out";

/// Create a directory if it doesn't yet exist, and return it as a
/// [PathBuf].
fn create_dir_if_not_exists<P>(path: P) -> eyre::Result<PathBuf>
where
    P: AsRef<Path> + Into<PathBuf> + std::fmt::Debug,
{
    if !path.as_ref().exists() {
        std::fs::create_dir(&path)
            .wrap_err_with(|| format!("Error while creating path {:?}.", path))?;
    }
    Ok(path.into())
}

#[derive(Debug, StructOpt)]
#[structopt(
    name = "Aleo Setup Integration Test",
    about = "An integration test for the aleo-setup and aleo-setup-coordinator repositories."
)]
struct Options {
    /// Remove any artifacts created during a previous integration
    /// test run before starting.
    #[structopt(long, short = "c")]
    clean: bool,

    /// Keep the git repositories. The following effects take place
    /// when this is enabled:
    ///
    /// + Don't delete git repositories if [Options::clean] is
    ///   enabled.
    #[structopt(long, short = "k")]
    keep_repos: bool,

    /// Don't attempt to install install prerequisites. Makes the test
    /// faster for development purposes.
    #[structopt(long, short = "n")]
    no_prereqs: bool,
}

/// Clone the git repos for `aleo-setup` and `aleo-setup-coordinator`.
fn clone_git_repos() -> eyre::Result<()> {
    clone_git_repository(COORDINATOR_REPO_URL, COORDINATOR_DIR, "main")
        .wrap_err("Error while cloning `aleo-setup-coordinator` git repository.")?;
    clone_git_repository(SETUP_REPO_URL, SETUP_DIR, "contributor-password-stdin")
        .wrap_err("Error while cloning the `aleo-setup` git repository.")?;

    Ok(())
}

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
fn main() -> eyre::Result<()> {
    setup_reporting()?;

    let options: Options = Options::from_args();

    let out_dir_path = Path::new(OUT_DIR);

    // Perfom the clean action if required.
    if options.clean {
        tracing::info!("Cleaning integration test.");
        
        if out_dir_path.exists() {
            tracing::info!("Removing out dir: {:?}", out_dir_path);
            std::fs::remove_dir_all(out_dir_path)?;
        }

        if !options.keep_repos {
            let setup_dir_path = Path::new(SETUP_DIR);
            if setup_dir_path.exists() {
                tracing::info!("Removing `aleo-setup` repository: {:?}.", setup_dir_path);
                std::fs::remove_dir_all(setup_dir_path)?;
            }

            let coordinator_dir_path = Path::new(COORDINATOR_DIR);
            if coordinator_dir_path.exists() {
                tracing::info!(
                    "Removing `aleo-setup-coordinator` repository: {:?}.",
                    coordinator_dir_path
                );
                std::fs::remove_dir_all(coordinator_dir_path)?;
            }
        }
    }

    let setup_phase = SetupPhase::Development;
    let out_dir_path = create_dir_if_not_exists(out_dir_path)?;
    let keys_dir_path = create_dir_if_not_exists(out_dir_path.join("keys"))?;
    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());

    if !options.no_prereqs {
        // Install a specific version of the rust toolchain needed to be
        // able to compile `aleo-setup`.
        install_rust_toolchain(&rust_1_47_nightly)?;
    }

    clone_git_repos()?;

    // Build the setup coordinator Rust project.
    build_rust_crate(COORDINATOR_DIR, &rust_1_47_nightly)?;
    let coordinator_bin_path = Path::new(COORDINATOR_DIR)
        .join("target/release")
        .join("aleo-setup-coordinator");

    let coordinator_config = CoordinatorConfig {
        crate_dir: PathBuf::from_str(COORDINATOR_DIR)?,
        setup_coordinator_bin: coordinator_bin_path,
        phase: setup_phase,
        out_dir_path: create_dir_if_not_exists(out_dir_path.join("coordinator"))?,
    };

    deploy_coordinator_rocket_config(&coordinator_config)?;

    if !options.no_prereqs {
        // Install the dependencies for the setup coordinator nodejs proxy.
        npm_install(COORDINATOR_DIR)?;
    }

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
    let setup_build_output_dir = Path::new(SETUP_DIR).join("target/release");

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let bus: Bus<CeremonyMessage> = Bus::new(1000);
    let ceremony_tx = bus.broadcaster();
    let ceremony_rx = bus.subscribe();

    let contributor_bin_path = setup_build_output_dir.join("aleo-setup-contributor");
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

    let round1_started = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundStarted(1)],
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
    let coordinator_proxy_out_dir =
        create_dir_if_not_exists(out_dir_path.join("coordinator_proxy"))?;
    let coordinator_proxy_join = run_coordinator_proxy(
        COORDINATOR_DIR,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        coordinator_proxy_out_dir,
    )?;

    // Run the coordinator.
    let coordinator_join = run_coordinator(
        &coordinator_config,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
    )?;

    // Wait for the coordinator and coordinator proxy to start.
    coordinator_ready
        .join()
        .wrap_err("Error while waiting for coordinator to start")?;

    tracing::info!("Coordinator started.");

    // Run the `setup1-contributor`.
    let contributor_out_dir = create_dir_if_not_exists(out_dir_path.join("contributor"))?;
    let contributor_join = run_contributor(
        contributor_bin_path,
        contributor1_key_file_path,
        setup_phase,
        COORDINATOR_API_URL.to_string(),
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        contributor_out_dir,
    )?;

    // Run the `setup1-verifier`.
    let verifier_bin_path = setup_build_output_dir.join("setup1-verifier");
    let verifier_out_dir = create_dir_if_not_exists(out_dir_path.join("verifier"))?;
    let verifier_join = run_verifier(
        verifier_bin_path,
        setup_phase,
        COORDINATOR_API_URL.to_string(),
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        verifier_out_dir,
    )?;

    tracing::info!("Waiting for round 1 to start.");

    round1_started
        .join()
        .wrap_err("Error while waiting for round 1 to finish")?;

    tracing::info!("Round 1 has started!");

    // TODO: currently this message isn't displayed until the
    // aggregation is complete. Perhaps this could be implemented by
    // checking the round's state.json file.
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
        .expect("unable to send shutdown message");

    // Wait for contributor threads to close after being told to shut down.
    contributor_join
        .join()
        .expect("Error while joining contributor threads.");

    // Wait for verifier threads to close after being told to shut down.
    verifier_join
        .join()
        .expect("Error while joining verifier threads.");

    // Wait for the coordinator threads to close after being told to shut down.
    coordinator_join
        .join()
        .expect("Error while joining coordinator threads.");

    // Wait for the coordinator proxy threads to close after being told to shut down.
    coordinator_proxy_join
        .join()
        .expect("Error while joining coordinator proxy threads.");

    Ok(())
}

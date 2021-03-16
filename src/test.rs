//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use crate::{
    contributor::{generate_contributor_key, run_contributor},
    coordinator::{deploy_coordinator_rocket_config, run_coordinator, CoordinatorConfig},
    coordinator_proxy::run_coordinator_proxy,
    git::clone_git_repository,
    npm::npm_install,
    options::CmdOptions,
    process::{join_multiple, MonitorProcessJoin},
    rust::{build_rust_crate, install_rust_toolchain, RustToolchain},
    verifier::run_verifier,
    CeremonyMessage, MessageWaiter, SetupPhase,
};
use eyre::Context;
use mpmc_bus::Bus;

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

/// The url for the `aleo-setup-coordinator` git repository.
const COORDINATOR_REPO_URL: &str = "git@github.com:AleoHQ/aleo-setup-coordinator.git";

/// The directory that the `aleo-setup-coordinator` repository is
/// cloned to.
const COORDINATOR_DIR: &str = "aleo-setup-coordinator";

/// The url for the `aleo-setup` git repository.
const SETUP_REPO_URL: &str = "git@github.com:AleoHQ/aleo-setup.git";

/// The directory that the `aleo-setup` repository is cloned to.
const SETUP_DIR: &str = "aleo-setup";

/// URL used by the contributors and verifiers to connect to the
/// coordinator.
const COORDINATOR_API_URL: &str = "http://localhost:9000";

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

/// Clone the git repos for `aleo-setup` and `aleo-setup-coordinator`.
pub fn clone_git_repos() -> eyre::Result<()> {
    clone_git_repository(COORDINATOR_REPO_URL, COORDINATOR_DIR, "main")
        .wrap_err("Error while cloning `aleo-setup-coordinator` git repository.")?;
    clone_git_repository(SETUP_REPO_URL, SETUP_DIR, "master")
        .wrap_err("Error while cloning the `aleo-setup` git repository.")?;
    Ok(())
}

struct Contributor {
    id: String,
    key_file: PathBuf,
}

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
pub fn run_integration_test(options: &CmdOptions) -> eyre::Result<()> {
    tracing::info!("Running integration test with options:\n{:#?}", &options);

    // Perfom the clean action if required.
    if options.clean {
        tracing::info!("Cleaning integration test.");

        if options.out_dir.exists() {
            tracing::info!("Removing out dir: {:?}", options.out_dir);
            std::fs::remove_dir_all(&options.out_dir)?;
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

    create_dir_if_not_exists(&options.out_dir)?;
    let test_config_path = options.out_dir.join("test_config.json");
    std::fs::write(test_config_path, serde_json::to_string_pretty(&options)?)?;

    let setup_phase = SetupPhase::Development;
    let keys_dir_path = create_dir_if_not_exists(options.out_dir.join("keys"))?;
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
        out_dir: create_dir_if_not_exists(options.out_dir.join("coordinator"))?,
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

    let contributors: Vec<Contributor> = (1..=options.contributors)
        .into_iter()
        .map(|i| {
            let id = format!("contributor{}", i);
            let contributor_key_file_name = format!("{}-key.json", id);
            let key_file = keys_dir_path.join(contributor_key_file_name);

            generate_contributor_key(&contributor_bin_path, &key_file)
                .wrap_err_with(|| format!("Error generating contributor {} key.", id))?;

            Ok(Contributor { id, key_file })
        })
        .collect::<eyre::Result<Vec<Contributor>>>()?;

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

    let mut joins: Vec<MonitorProcessJoin> = Vec::new();

    // Run the nodejs proxy server for the coordinator.
    let coordinator_proxy_out_dir =
        create_dir_if_not_exists(options.out_dir.join("coordinator_proxy"))?;
    let coordinator_proxy_join = run_coordinator_proxy(
        COORDINATOR_DIR,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        coordinator_proxy_out_dir,
    )?;
    joins.push(coordinator_proxy_join);

    // Run the coordinator.
    let coordinator_join = run_coordinator(
        &coordinator_config,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
    )?;
    joins.push(coordinator_join);

    // Wait for the coordinator and coordinator proxy to start.
    coordinator_ready
        .join()
        .wrap_err("Error while waiting for coordinator to start")?;

    tracing::info!("Coordinator started.");

    for contributor in contributors {
        // Run the `setup1-contributor`.
        let contributor_out_dir = create_dir_if_not_exists(options.out_dir.join(&contributor.id))?;
        let contributor_join = run_contributor(
            contributor_bin_path.clone(),
            contributor.key_file,
            setup_phase,
            COORDINATOR_API_URL.to_string(),
            ceremony_tx.clone(),
            ceremony_rx.clone(),
            contributor_out_dir,
        )?;
        joins.push(contributor_join);
    }

    // Run the `setup1-verifier`.
    let verifier_bin_path = setup_build_output_dir.join("setup1-verifier");
    let verifier_out_dir = create_dir_if_not_exists(options.out_dir.join("verifier"))?;
    let verifier_join = run_verifier(
        verifier_bin_path,
        setup_phase,
        COORDINATOR_API_URL.to_string(),
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        verifier_out_dir,
    )?;
    joins.push(verifier_join);

    tracing::info!("Waiting for round 1 to start.");

    round1_started
        .join()
        .wrap_err("Error while waiting for round 1 to start")?;

    tracing::info!("Round 1 has started!");

    // TODO: currently this message isn't displayed until the
    // aggregation is complete. Perhaps this could be implemented by
    // checking the round's state.json file.
    round1_finished
        .join()
        .wrap_err("Error while waiting for round 1 to finish.")?;

    tracing::info!("Round 1 Finished (waiting for aggregation to complete).");

    round1_aggregated
        .join()
        .wrap_err("Error while waiting for round 1 to aggregate.")?;

    tracing::info!("Round 1 Aggregated, test complete.");

    // Tell the other threads to shutdown, safely terminating their
    // child processes.
    ceremony_tx
        .broadcast(CeremonyMessage::Shutdown)
        .expect("Unable to send shutdown message.");

    // Wait for monitor threads to close after being told to shut down.
    join_multiple(joins).expect("Error while joining monitor threads.");
    Ok(())
}

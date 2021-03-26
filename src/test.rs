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
    state_monitor::{run_state_monitor, setup_state_monitor},
    time_limit::start_ceremony_time_limit,
    util::create_dir_if_not_exists,
    verifier::{generate_verifier_key, run_verifier, VerifierViewKey},
    CeremonyMessage, Environment, MessageWaiter, WaiterJoinCondition,
};

use eyre::Context;
use humantime::format_duration;
use mpmc_bus::Bus;
use serde::{Deserialize, Serialize};

use std::{
    convert::TryFrom,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Command line options for running the Aleo Setup integration test.
#[derive(Debug, Serialize, Deserialize)]
pub struct TestOptions {
    /// Remove any artifacts created during a previous integration
    /// test run before starting.
    pub clean: bool,

    /// Keep the git repositories. The following effects take place
    /// when this is enabled:
    ///
    /// + Don't delete git repositories if [Options::clean] is
    ///   enabled.
    pub keep_repos: bool,

    /// If true, don't attempt to install install prerequisites. Makes
    /// the test faster for development purposes.
    pub no_prereqs: bool,

    /// Number of contributor participants for the test.
    pub contributors: u8,

    /// Number of verifier participants for the test.
    pub verifiers: u8,

    /// Path to where the log files, key files and transcripts are stored.
    pub out_dir: PathBuf,

    /// What environment to use for the setup.
    pub environment: Environment,

    /// Whether to run the `aleo-setup-state-monitor` application.
    /// Requires `python3` and `pip` to be installed. Only supported
    /// on Linux.
    pub state_monitor: bool,

    /// Timout (in seconds) for running a ceremony round of the
    /// integration test (not including setting up prerequisites). If
    /// this time is exceeded for a given round, the test will fail.
    pub round_timout: Option<std::time::Duration>,
}

impl TryFrom<&CmdOptions> for TestOptions {
    type Error = eyre::Error;

    fn try_from(options: &CmdOptions) -> Result<Self, Self::Error> {
        Ok(Self {
            clean: options.clean,
            keep_repos: options.keep_repos,
            no_prereqs: options.no_prereqs,
            contributors: options.contributors,
            verifiers: options.verifiers,
            out_dir: options.out_dir.clone(),
            environment: options.environment,
            state_monitor: options.state_monitor,
            round_timout: options.round_timeout,
        })
    }
}

#[derive(Serialize)]
pub struct TestResults {
    /// The time between the start of the round, and the end of the
    /// round.
    #[serde(with = "humantime_serde")]
    pub total_round_duration: std::time::Duration,
    /// The time taken to perform aggregation at the end of a round.
    #[serde(with = "humantime_serde")]
    pub aggregation_duration: std::time::Duration,
}

/// The url for the `aleo-setup-coordinator` git repository.
const COORDINATOR_REPO_URL: &str = "git@github.com:AleoHQ/aleo-setup-coordinator.git";

/// The directory that the `aleo-setup-coordinator` repository is
/// cloned to.
const COORDINATOR_DIR: &str = "aleo-setup-coordinator";

/// The url for the `aleo-setup-status-monitor` git repository.
const STATE_MONITOR_REPO_URL: &str = "git@github.com:AleoHQ/aleo-setup-state-monitor.git";

/// The directory that the `aleo-setup-state-monitor` repository is
/// cloned to.
const STATE_MONITOR_DIR: &str = "aleo-setup-state-monitor";

/// The url for the `aleo-setup` git repository.
const SETUP_REPO_URL: &str = "git@github.com:AleoHQ/aleo-setup.git";

/// The directory that the `aleo-setup` repository is cloned to.
const SETUP_DIR: &str = "aleo-setup";

/// URL used by the contributors and verifiers to connect to the
/// coordinator.
const COORDINATOR_API_URL: &str = "http://localhost:9000";

/// Clone the git repos for `aleo-setup` and `aleo-setup-coordinator`.
pub fn clone_git_repos(options: &TestOptions) -> eyre::Result<()> {
    clone_git_repository(COORDINATOR_REPO_URL, COORDINATOR_DIR, "main")
        .wrap_err("Error while cloning `aleo-setup-coordinator` git repository.")?;
    clone_git_repository(SETUP_REPO_URL, SETUP_DIR, "master")
        .wrap_err("Error while cloning the `aleo-setup` git repository.")?;

    if options.state_monitor {
        clone_git_repository(STATE_MONITOR_REPO_URL, STATE_MONITOR_DIR, "include-build")
            .wrap_err("Error while cloning `aleo-setup-state-monitor` git repository.")?;
    }

    Ok(())
}

/// Create a bash script in the `out_dir` called `tail-logs.sh` which
/// sets up a tmux session to view the logs using `tail` in real-time.
fn write_tail_logs_script(
    out_dir: impl AsRef<Path>,
    contributors: &[Contributor],
    verifiers: &[Verifier],
) -> eyre::Result<()> {
    let mut file = File::create(out_dir.as_ref().join("tail-logs.sh"))
        .wrap_err("Unable to create `tail-logs.sh`")?;

    file.write_all("#!/bin/sh\n".as_bytes())?;
    file.write_all(
        "tmux new-session -s test -n \"Coordinator\" -d 'tail -f coordinator/coordinator.log'\n"
            .as_bytes(),
    )?;
    file.write_all("tmux new-window -t test:1 -n \"Coordinator Proxy\" 'tail -f coordinator_proxy/coordinator_proxy.log'\n".as_bytes())?;

    let mut window_index: u8 = 2;

    for contributor in contributors {
        let command = format!(
            "tmux new-window -t test:{0} -n \"{1}\" 'tail -f \"{1}/contributor.log\"'\n",
            window_index, contributor.id
        );
        file.write_all(command.as_bytes())?;
        window_index += 1;
    }

    for verifier in verifiers {
        let command = format!(
            "tmux new-window -t test:{0} -n \"{1}\" 'tail -f \"{1}/verifier.log\"'\n",
            window_index, verifier.id
        );
        file.write_all(command.as_bytes())?;
        window_index += 1;
    }

    file.write_all("tmux select-window -t test:0\ntmux -2 attach-session -t test\n".as_ref())?;

    Ok(())
}

/// Data relating to a contributor.
struct Contributor {
    id: String,
    key_file: PathBuf,
}

/// Data relating to a verifier.
struct Verifier {
    id: String,
    view_key: VerifierViewKey,
}

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
pub fn run_integration_test(options: &TestOptions) -> eyre::Result<TestResults> {
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

    let keys_dir_path = create_dir_if_not_exists(options.out_dir.join("keys"))?;
    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());

    clone_git_repos(&options)?;

    if !options.no_prereqs {
        // Install a specific version of the rust toolchain needed to be
        // able to compile `aleo-setup`.
        install_rust_toolchain(&rust_1_47_nightly)?;
        // Install the dependencies for the setup coordinator nodejs proxy.
        npm_install(COORDINATOR_DIR)?;
        if options.state_monitor {
            setup_state_monitor(STATE_MONITOR_DIR)?;
        }
    }

    // Build the setup coordinator Rust project.
    build_rust_crate(COORDINATOR_DIR, &rust_1_47_nightly)?;
    let coordinator_bin_path = Path::new(COORDINATOR_DIR)
        .join("target/release")
        .join("aleo-setup-coordinator");

    let coordinator_config = CoordinatorConfig {
        crate_dir: PathBuf::from_str(COORDINATOR_DIR)?,
        setup_coordinator_bin: coordinator_bin_path,
        environment: options.environment,
        out_dir: create_dir_if_not_exists(options.out_dir.join("coordinator"))?,
    };

    deploy_coordinator_rocket_config(&coordinator_config)?;

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

    // Build the setup1-cli-tools Rust project.
    build_rust_crate(
        Path::new(SETUP_DIR).join("setup1-cli-tools"),
        &rust_1_47_nightly,
    )?;

    // Output directory for setup1-verifier and setup1-contributor
    // projects.
    let setup_build_output_dir = Path::new(SETUP_DIR).join("target/release");

    let view_key_bin_path = setup_build_output_dir.join("view-key");

    // Create the verifiers, generate their keys.
    let verifiers: Vec<Verifier> = (1..=options.verifiers)
        .into_iter()
        .map(|i| {
            let id = format!("verifier{}", i);
            let view_key = generate_verifier_key(&view_key_bin_path)?;

            Ok(Verifier { id, view_key })
        })
        .collect::<eyre::Result<Vec<Verifier>>>()?;

    let contributor_bin_path = setup_build_output_dir.join("aleo-setup-contributor");

    // Create the contributors, generate their keys.
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

    write_tail_logs_script(&options.out_dir, &contributors, &verifiers)?;

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let bus: Bus<CeremonyMessage> = Bus::new(1000);
    let ceremony_tx = bus.broadcaster();
    let ceremony_rx = bus.subscribe();

    let time_limit_join = options.round_timout.map(|timeout| {
        start_ceremony_time_limit(timeout, ceremony_tx.clone(), ceremony_rx.clone())
    });

    // Watches the bus to determine when the coordinator and coordinator proxy are ready.
    let coordinator_ready = MessageWaiter::spawn(
        vec![
            CeremonyMessage::RoundWaitingForParticipants(1),
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

    let round1_aggregation_started = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundStartedAggregation(1)],
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
    let coordinator_run_details = run_coordinator(
        &coordinator_config,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
    )?;
    let coordinator_transcript_dir = coordinator_run_details.transcript_dir.clone();
    joins.push(coordinator_run_details.join);

    if options.state_monitor {
        let state_monitor_join = run_state_monitor(
            STATE_MONITOR_DIR,
            &coordinator_transcript_dir,
            ceremony_tx.clone(),
            ceremony_rx.clone(),
            &options.out_dir,
        )?;
        joins.push(state_monitor_join);
    }

    // Wait for the coordinator and coordinator proxy to start.
    coordinator_ready
        .join()
        .wrap_err("Error while waiting for coordinator to start")?;

    tracing::info!("Coordinator started.");

    for contributor in contributors {
        // Run the `setup1-contributor`.
        let contributor_out_dir = create_dir_if_not_exists(options.out_dir.join(&contributor.id))?;
        let contributor_join = run_contributor(
            &contributor.id,
            contributor_bin_path.clone(),
            contributor.key_file,
            options.environment,
            COORDINATOR_API_URL,
            ceremony_tx.clone(),
            ceremony_rx.clone(),
            contributor_out_dir,
        )?;
        joins.push(contributor_join);
    }

    for verifier in verifiers {
        // Run the `setup1-verifier`.
        let verifier_bin_path = setup_build_output_dir.join("setup1-verifier");
        let verifier_out_dir = create_dir_if_not_exists(options.out_dir.join(&verifier.id))?;
        let verifier_join = run_verifier(
            &verifier.id,
            verifier_bin_path,
            options.environment,
            COORDINATOR_API_URL,
            &verifier.view_key,
            ceremony_tx.clone(),
            ceremony_rx.clone(),
            verifier_out_dir,
        )?;
        joins.push(verifier_join);
    }

    let round_start_time = std::time::Instant::now();

    tracing::info!("Waiting for round 1 to start.");

    round1_started
        .join()
        .wrap_err("Error while waiting for round 1 to start")?
        .on_messages_received(|| tracing::info!("Round 1 has started!"));

    round1_aggregation_started
        .join()
        .wrap_err("Error while waiting for round aggregation 1 to start")?
        .on_messages_received(|| {
            tracing::info!(
                "Round 1 contributions and verifications complete. Aggregation has started."
            )
        });

    let aggregation_start_time = std::time::Instant::now();

    let aggregation_duration = match round1_aggregated
        .join()
        .wrap_err("Error while waiting for round 1 to aggregate.")?
    {
        WaiterJoinCondition::Shutdown => None,
        WaiterJoinCondition::MessagesReceived => {
            tracing::info!("Round 1 Aggregated.");
            let aggregation_duration = aggregation_start_time.elapsed();
            tracing::info!(
                "Aggregation time: {}",
                format_duration(aggregation_duration.clone())
            );
            Some(aggregation_duration)
        }
    };

    let total_round_duration = match round1_finished
        .join()
        .wrap_err("Error while waiting for round 1 to finish.")?
    {
        WaiterJoinCondition::Shutdown => None,
        WaiterJoinCondition::MessagesReceived => {
            tracing::info!("Round 1 Finished.");
            let total_round_duration = round_start_time.elapsed();
            tracing::info!(
                "Total round time: {}",
                format_duration(total_round_duration.clone())
            );
            Some(total_round_duration)
        }
    };

    // Tell the other threads to shutdown, safely terminating their
    // child processes.
    ceremony_tx
        .broadcast(CeremonyMessage::Shutdown)
        .expect("Unable to send shutdown message.");

    // Wait for threads to close after being told to shut down.
    join_multiple(joins).expect("Error while joining monitor threads.");

    // Joining time limit after the other threads because currnently
    // it panics on an error, and want the other threads to have time
    // to close gracefully.
    match time_limit_join {
        Some(handle) => {
            handle
                .join()
                .expect("error while joining time limit thread")?;
        }
        None => {}
    }

    tracing::info!("All threads/processes joined, test complete!");

    let results = TestResults {
        total_round_duration: total_round_duration
            .unwrap_or_else(|| std::time::Duration::from_secs(0)),
        aggregation_duration: aggregation_duration
            .unwrap_or_else(|| std::time::Duration::from_secs(0)),
    };

    std::fs::write(
        options.out_dir.join("results.json"),
        serde_json::to_string_pretty(&results)?,
    )?;

    Ok(results)
}

//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use crate::{
    contributor::{generate_contributor_key, run_contributor, Contributor, ContributorConfig},
    coordinator::{check_participants_in_round, run_coordinator, CoordinatorConfig},
    coordinator_proxy::run_coordinator_proxy,
    drop_participant::{monitor_drops, DropContributorConfig, MonitorDropsConfig},
    git::{clone_git_repository, LocalGitRepo, RemoteGitRepo},
    npm::npm_install,
    options::CmdOptions,
    process::{join_multiple, MultiJoinable},
    reporting::LogFileWriter,
    rust::{build_rust_crate, install_rust_toolchain, RustToolchain},
    state_monitor::{run_state_monitor, setup_state_monitor},
    time_limit::ceremony_time_limit,
    util::create_dir_if_not_exists,
    verifier::{generate_verifier_key, run_verifier, Verifier},
    CeremonyMessage, ContributorRef, Environment, MessageWaiter, ShutdownReason,
    WaiterJoinCondition,
};

use eyre::Context;
use humantime::format_duration;
use mpmc_bus::Bus;
use serde::{Deserialize, Serialize};

use std::{
    collections::HashMap,
    convert::TryFrom,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

/// Code repository to be used during a test.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum Repo {
    /// A local git repository, already present on the file system.
    Local(LocalGitRepo),
    /// A remote git repository to be cloned.
    Remote(RemoteGitRepo),
}

impl Repo {
    pub fn dir(&self) -> &Path {
        match self {
            Repo::Local(repo) => &repo.dir,
            Repo::Remote(repo) => &repo.dir,
        }
    }
}

/// Default repository specification for the `aleo-setup` project.
pub fn default_aleo_setup() -> Repo {
    Repo::Remote(RemoteGitRepo {
        dir: "aleo-setup".into(),
        url: "git@github.com:AleoHQ/aleo-setup.git".into(),
        branch: "master".into(),
    })
}

/// Default repository specification for the `aleo-setup-coordinator` project.
pub fn default_aleo_setup_coordinator() -> Repo {
    Repo::Remote(RemoteGitRepo {
        dir: "aleo-setup-coordinator".into(),
        url: "git@github.com:AleoHQ/aleo-setup-coordinator.git".into(),
        branch: "main".into(),
    })
}

/// Default repository specification for the `aleo-setup-state-monitor` project.
pub fn default_aleo_setup_state_monitor() -> Repo {
    Repo::Remote(RemoteGitRepo {
        dir: "aleo-setup-state-monitor".into(),
        url: "git@github.com:AleoHQ/aleo-setup-state-monitor.git".into(),
        branch: "include-build".into(), // branch to include build files so that npm is not required
    })
}

/// Command line options for running the Aleo Setup integration test.
#[derive(Debug, Serialize)]
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

    /// Number of replacement contributors for the test.
    pub replacement_contributors: u8,

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

    /// Configuration for dropping contributors during the ceremony.
    pub contributor_drops: Vec<DropContributorConfig>,

    /// The code repository for the `aleo-setup` project.
    pub aleo_setup_repo: Repo,

    /// The code repository for the `aleo-setup-coordinator` project.
    pub aleo_setup_coordinator_repo: Repo,

    /// The code repository for the `aleo-setup-state-monitor` project.
    pub aleo_setup_state_monitor_repo: Repo,
}

impl TryFrom<&CmdOptions> for TestOptions {
    type Error = eyre::Error;

    fn try_from(options: &CmdOptions) -> Result<Self, Self::Error> {
        Ok(Self {
            clean: options.clean,
            keep_repos: options.keep_repos,
            no_prereqs: options.no_prereqs,
            contributors: options.contributors,
            replacement_contributors: options.replacement_contributors,
            verifiers: options.verifiers,
            out_dir: options.out_dir.clone(),
            environment: options.environment,
            state_monitor: options.state_monitor,
            round_timout: options.round_timeout,
            contributor_drops: Vec::new(),
            aleo_setup_repo: options
                .aleo_setup_repo
                .clone()
                .map(|dir| Repo::Local(LocalGitRepo { dir }))
                .unwrap_or_else(default_aleo_setup),
            aleo_setup_coordinator_repo: options
                .aleo_setup_coordinator_repo
                .clone()
                .map(|dir| Repo::Local(LocalGitRepo { dir }))
                .unwrap_or_else(default_aleo_setup_coordinator),
            aleo_setup_state_monitor_repo: options
                .aleo_setup_state_monitor_repo
                .clone()
                .map(|dir| Repo::Local(LocalGitRepo { dir }))
                .unwrap_or_else(default_aleo_setup_state_monitor),
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

/// URL used by the contributors and verifiers to connect to the
/// coordinator.
const COORDINATOR_API_URL: &str = "http://localhost:9000";

/// Clone the git repos for `aleo-setup` and `aleo-setup-coordinator`.
pub fn clone_git_repos(options: &TestOptions) -> eyre::Result<()> {
    if let Repo::Remote(repo) = &options.aleo_setup_coordinator_repo {
        clone_git_repository(repo)
            .wrap_err("Error while cloning `aleo-setup-coordinator` git repository.")?;
    }

    if let Repo::Remote(repo) = &options.aleo_setup_repo {
        clone_git_repository(repo).wrap_err("Error while cloning `aleo-setup` git repository.")?;
    }

    if options.state_monitor {
        if let Repo::Remote(repo) = &options.aleo_setup_state_monitor_repo {
            clone_git_repository(repo)
                .wrap_err("Error while cloning `aleo-setup-state-monitor` git repository.")?;
        }
    }

    Ok(())
}

/// Create a bash script in the `out_dir` called `tail-logs.sh` which
/// sets up a tmux session to view the logs using `tail` in real-time.
fn write_tail_logs_script<'c>(
    out_dir: impl AsRef<Path>,
    contributors: impl IntoIterator<Item = &'c Contributor>,
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

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
pub fn run_integration_test(
    options: &TestOptions,
    log_writer: &LogFileWriter,
) -> eyre::Result<TestResults> {
    log_writer.set_no_out_file();
    tracing::info!("Running integration test with options:\n{:#?}", &options);

    // Perfom the clean action if required.
    if options.clean {
        tracing::info!("Cleaning integration test.");

        if options.out_dir.exists() {
            tracing::info!("Removing out dir: {:?}", options.out_dir);
            std::fs::remove_dir_all(&options.out_dir)?;
        }

        if !options.keep_repos {
            if let Repo::Remote(repo) = &options.aleo_setup_repo {
                if repo.dir.exists() {
                    tracing::info!("Removing `aleo-setup` repository: {:?}.", &repo.dir);
                    std::fs::remove_dir_all(&repo.dir)?;
                }
            }

            if let Repo::Remote(repo) = &options.aleo_setup_coordinator_repo {
                if repo.dir.exists() {
                    tracing::info!(
                        "Removing `aleo-setup-coordinator` repository: {:?}.",
                        &repo.dir
                    );
                    std::fs::remove_dir_all(&repo.dir)?;
                }
            }
        }
    }

    // Create the log file, and write out the options that were used to run this test.
    create_dir_if_not_exists(&options.out_dir)?;
    log_writer.set_out_file(&options.out_dir.join("integration-test.log"))?;
    let test_config_path = options.out_dir.join("test_config.ron");
    std::fs::write(test_config_path, ron::ser::to_string_pretty(&options, Default::default())?)?;

    // Directory to store the contributor and verifier keys.
    let keys_dir_path = create_dir_if_not_exists(options.out_dir.join("keys"))?;

    // Unfortunately aleo-setup still requires an old version of nightly to compile.
    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());

    // Attempt to clone the git repos if they don't already exist.
    clone_git_repos(&options)?;

    let coordinator_dir = options.aleo_setup_coordinator_repo.dir();
    let state_monitor_dir = options.aleo_setup_state_monitor_repo.dir();
    let setup_dir = options.aleo_setup_repo.dir();

    if !options.no_prereqs {
        // Install a specific version of the rust toolchain needed to be
        // able to compile `aleo-setup`.
        install_rust_toolchain(&rust_1_47_nightly)?;
        // Install the dependencies for the setup coordinator nodejs proxy.
        npm_install(coordinator_dir)?;
        if options.state_monitor {
            setup_state_monitor(state_monitor_dir)?;
        }
    }

    // Build the setup coordinator Rust project.
    build_rust_crate(coordinator_dir, &rust_1_47_nightly)?;
    let coordinator_bin_path = Path::new(coordinator_dir)
        .join("target/release")
        .join("aleo-setup-coordinator");

    // Build the setup1-contributor Rust project.
    build_rust_crate(setup_dir.join("setup1-contributor"), &rust_1_47_nightly)?;

    // Build the setup1-verifier Rust project.
    build_rust_crate(setup_dir.join("setup1-verifier"), &rust_1_47_nightly)?;

    // Build the setup1-cli-tools Rust project.
    build_rust_crate(setup_dir.join("setup1-cli-tools"), &rust_1_47_nightly)?;

    // Output directory for setup1-verifier and setup1-contributor
    // projects.
    let setup_build_output_dir = setup_dir.join("target/release");

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

            let contributor_key = generate_contributor_key(&contributor_bin_path, &key_file)
                .wrap_err_with(|| format!("Error generating contributor {} key.", id))?;

            Ok(Contributor {
                id,
                key_file,
                address: contributor_key.address,
            })
        })
        .collect::<eyre::Result<Vec<Contributor>>>()?;

    // Create the replacement contributors, generate their keys.
    let replacement_contributors: Vec<Contributor> = (1..=options.replacement_contributors)
        .into_iter()
        .map(|i| {
            let id = format!("replacement_contributor{}", i);
            let contributor_key_file_name = format!("{}-key.json", id);
            let key_file = keys_dir_path.join(contributor_key_file_name);

            let contributor_key = generate_contributor_key(&contributor_bin_path, &key_file)
                .wrap_err_with(|| format!("Error generating contributor {} key.", id))?;

            Ok(Contributor {
                id,
                key_file,
                address: contributor_key.address,
            })
        })
        .collect::<eyre::Result<Vec<Contributor>>>()?;

    let replacement_contributor_refs: Vec<ContributorRef> = replacement_contributors
        .iter()
        .map(|c| c.as_contributor_ref())
        .collect();

    let coordinator_config = CoordinatorConfig {
        crate_dir: coordinator_dir.to_owned(),
        setup_coordinator_bin: coordinator_bin_path,
        environment: options.environment,
        out_dir: create_dir_if_not_exists(options.out_dir.join("coordinator"))?,
        replacement_contributors: replacement_contributor_refs,
    };

    write_tail_logs_script(
        &options.out_dir,
        contributors.iter().chain(replacement_contributors.iter()),
        &verifiers,
    )?;

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let bus: Bus<CeremonyMessage> = Bus::new(1000);
    let ceremony_tx = bus.broadcaster();
    let ceremony_rx = bus.subscribe();

    let time_limit_join = options
        .round_timout
        .map(|timeout| ceremony_time_limit(timeout, ceremony_rx.clone(), ceremony_tx.clone()));

    let contributor_drops = options
        .contributor_drops
        .iter()
        .enumerate()
        .map(|(i, drop_config)| {
            let contributor_ref: ContributorRef = contributors
                .get(i)
                .ok_or_else(|| {
                    eyre::eyre!(
                        "There is no contributor corresponding to the drop config at index {}",
                        i
                    )
                })?
                .as_contributor_ref();
            Ok((contributor_ref, drop_config.clone()))
        })
        .collect::<eyre::Result<HashMap<ContributorRef, DropContributorConfig>>>()?;

    // Monitor the ceremony for dropped participants
    let drops_config = MonitorDropsConfig {
        contributor_drops: contributor_drops.clone(),
    };
    let monitor_drops_join = monitor_drops(drops_config, ceremony_rx.clone(), ceremony_tx.clone());

    // Construct MessageWaiters which wait for specific messages
    // during the ceremony before joining.
    let coordinator_ready = MessageWaiter::spawn(
        vec![
            CeremonyMessage::RoundWaitingForParticipants(1),
            CeremonyMessage::CoordinatorProxyReady,
        ],
        CeremonyMessage::is_shutdown,
        ceremony_rx.clone(),
    );
    let round1_started = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundStarted(1)],
        CeremonyMessage::is_shutdown,
        ceremony_rx.clone(),
    );
    let round1_aggregation_started = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundStartedAggregation(1)],
        CeremonyMessage::is_shutdown,
        ceremony_rx.clone(),
    );
    let round1_finished = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundFinished(1)],
        CeremonyMessage::is_shutdown,
        ceremony_rx.clone(),
    );
    let round1_aggregated = MessageWaiter::spawn(
        vec![CeremonyMessage::RoundAggregated(1)],
        CeremonyMessage::is_shutdown,
        ceremony_rx.clone(),
    );

    let mut process_joins: Vec<Box<dyn MultiJoinable>> = Vec::new();

    // Run the nodejs proxy server for the coordinator.
    let coordinator_proxy_out_dir =
        create_dir_if_not_exists(options.out_dir.join("coordinator_proxy"))?;
    let coordinator_proxy_join = run_coordinator_proxy(
        coordinator_dir,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
        coordinator_proxy_out_dir,
    )?;
    process_joins.push(Box::new(coordinator_proxy_join));

    // Run the coordinator.
    let coordinator_join = run_coordinator(
        &coordinator_config,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
    )?;

    process_joins.push(Box::new(coordinator_join));

    if options.state_monitor {
        let state_monitor_join = run_state_monitor(
            state_monitor_dir,
            &coordinator_config.transcript_dir(),
            ceremony_tx.clone(),
            ceremony_rx.clone(),
            &options.out_dir,
        )?;
        process_joins.push(Box::new(state_monitor_join));
    }

    // Wait for the coordinator and coordinator proxy to start.
    coordinator_ready
        .join()
        .wrap_err("Error while waiting for coordinator to start")?;

    tracing::info!("Coordinator started.");

    // Run the contributors and replacement contributors.
    for contributor in contributors.iter().chain(replacement_contributors.iter()) {
        // Run the `setup1-contributor`.
        let contributor_out_dir = create_dir_if_not_exists(options.out_dir.join(&contributor.id))?;
        let drop = contributor_drops
            .get(&contributor.as_contributor_ref())
            .cloned();
        let contributor_config = ContributorConfig {
            id: contributor.id.clone(),
            contributor_ref: contributor.as_contributor_ref(),
            contributor_bin_path: contributor_bin_path.clone(),
            key_file_path: contributor.key_file.clone(),
            environment: options.environment,
            coordinator_api_url: COORDINATOR_API_URL.to_string(),
            out_dir: contributor_out_dir,
            drop,
        };

        let contributor_join =
            run_contributor(contributor_config, ceremony_tx.clone(), ceremony_rx.clone())?;
        process_joins.push(Box::new(contributor_join));
    }

    for verifier in &verifiers {
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
        process_joins.push(Box::new(verifier_join));
    }

    let mut round_errors: Vec<eyre::Error> = Vec::new();

    let round_start_time = std::time::Instant::now();

    tracing::info!("Waiting for round 1 to start.");

    match round1_started
        .join()
        .wrap_err("Error while waiting for round 1 to start")?
    {
        WaiterJoinCondition::Shutdown => {}
        WaiterJoinCondition::MessagesReceived => {
            tracing::info!("Round 1 has started!");

            if let Err(error) =
                check_participants_in_round(&coordinator_config, 1, &contributors, &verifiers)
            {
                ceremony_tx.broadcast(CeremonyMessage::Shutdown(ShutdownReason::Error))?;
                round_errors.push(error);
            }
        }
    }

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
    ceremony_tx.broadcast(CeremonyMessage::Shutdown(ShutdownReason::TestFinished))?;

    // Wait for threads to close after being told to shut down.
    join_multiple(process_joins).expect("Error while joining monitor threads.");

    monitor_drops_join
        .join()
        .expect("Error while monitor drops thread")?;

    match time_limit_join {
        Some(handle) => {
            tracing::debug!("Waiting for time limit to join");
            if let Err(error) = handle
                .join()
                .expect("error while joining time limit thread")
            {
                tracing::error!("{:?}", error);
                round_errors.push(error);
            }
        }
        None => {}
    }

    tracing::info!("All threads/processes joined, test complete!");

    if !round_errors.is_empty() {
        tracing::error!("Round completed with errors.");
        for error in &round_errors {
            tracing::error!("{:?}", error);
        }

        return Err(round_errors
            .pop()
            .expect("expected one error to be present"));
    }

    let results = TestResults {
        total_round_duration: total_round_duration
            .unwrap_or_else(|| std::time::Duration::from_secs(0)),
        aggregation_duration: aggregation_duration
            .unwrap_or_else(|| std::time::Duration::from_secs(0)),
    };

    std::fs::write(
        options.out_dir.join("results.ron"),
        ron::ser::to_string_pretty(&results, Default::default())?,
    )?;

    Ok(results)
}

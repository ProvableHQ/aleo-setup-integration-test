//! Integration test for `aleo contributors: (), contributor_drops: ()  contributors: (), contributor_drops: () -setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use crate::{
    ceremony_waiter::spawn_contribution_waiter,
    cli_contributor::{
        generate_contributor_key, run_cli_contributor, CLIContributor, CLIContributorConfig,
    },
    coordinator::{check_participants_in_round, run_coordinator, CoordinatorConfig},
    drop_participant::{monitor_drops, DropContributorConfig, MonitorDropsConfig},
    git::{LocalGitRepo, RemoteGitRepo},
    join::{join_multiple, JoinLater, JoinMultiple, MultiJoinable},
    reporting::LogFileWriter,
    state_monitor::{run_state_monitor, StateMonitorConfig},
    time_limit::ceremony_time_limit,
    util::create_dir_if_not_exists,
    verifier::{generate_verifier_key, run_verifier, Verifier},
    waiter::{MessageWaiter, WaiterJoinCondition},
    CeremonyMessage, ContributorRef, Environment, ShutdownReason,
};

use eyre::Context;
use humantime::format_duration;
use mpmc_bus::{Bus, Receiver, Sender};
use serde::{Deserialize, Serialize};

use std::{
    collections::HashMap,
    net::SocketAddr,
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

/// Start a ceremony participant after
/// [StartAfterContributions::contributions] have been made in the
/// current round.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartAfterRoundContributions {
    /// See [StartAfterContributions].
    after_round_contributions: u64,
}

/// The configuration for when a contributor will be started
/// during/before a round.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContributorStartConfig {
    /// Start the contributor at the beginning of the ceremony. This
    /// is only a valid option for replacement contributors.
    CeremonyStart,
    /// Start the contributor while the current round is waiting for
    /// participants to join.
    RoundStart,
    // See [StartAfterContributions].
    AfterRoundContributions(StartAfterRoundContributions),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestRound {
    /// Number of CLI contributor participants for this round of the
    /// ceremony. By default the contributor will be started at the
    /// start of the round as per
    /// [ContributorStartConfig::RoundStart], however you may choose
    /// to override this for contributors with
    /// [TestRound::contributor_starts].
    #[serde(default)]
    pub cli_contributors: u8,

    /// Number of browser contributor participants for this round of
    /// the ceremony. By default the contributor will be started at
    /// the start of the round as per
    /// [ContributorStartConfig::RoundStart], however you may choose
    /// to override this for contributors with
    /// [TestRound::contributor_starts].
    #[serde(default)]
    pub browser_contributors: u8,

    /// (Optional) Configure expected contributor drops. A contributor from
    /// [Self::contributors] is assigned automatically to each
    /// specified config. The number of configs should not exceed the
    /// number of contributors. Default: [].
    #[serde(default)]
    pub contributor_drops: Vec<DropContributorConfig>,

    /// (Optional) Configure when contributors will start. A
    /// contributor from [Self::contributors] is assigned
    /// automatically to each specified config. The number of configs
    /// should not exceed the number of contributors. Any contributors
    /// not configured here will be started with the start of the
    /// round as per [ContributorStartConfig::RoundStart]. Default:
    /// [].
    #[serde(default)]
    pub contributor_starts: Vec<ContributorStartConfig>,
}

impl Default for TestRound {
    fn default() -> Self {
        Self {
            cli_contributors: 1,
            browser_contributors: 0,
            contributor_drops: Default::default(),
            contributor_starts: Default::default(),
        }
    }
}

/// Command line options for running the Aleo Setup integration test.
#[derive(Debug, Serialize)]
pub struct TestOptions {
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
    pub state_monitor: Option<StateMonitorOptions>,

    /// Timout for this individual integration test (not including
    /// setting up prerequisites). If this time is exceeded, the test
    /// will fail.
    pub timout: Option<std::time::Duration>,

    /// The code repository for the `aleo-setup` project.
    pub aleo_setup_repo: Repo,

    /// The code repository for the `aleo-setup-coordinator` project.
    pub aleo_setup_coordinator_repo: Repo,

    /// The code repository for the `setup-frontend`
    pub setup_frontend_repo: Repo,

    /// Configuration for each round of the ceremony that will be tested.
    pub rounds: Vec<TestRound>,
}

/// Options for running the `aleo-setup-state-monitor`
#[derive(Debug, Serialize)]
pub struct StateMonitorOptions {
    /// The code repository for the `aleo-setup-state-monitor` project.
    pub repo: Repo,
    /// The address used for the `aleo-setup-state-monitor` web
    /// server.
    pub address: SocketAddr,
}

#[derive(Serialize)]
pub struct RoundResults {
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

#[derive(Serialize)]
pub struct TestResults {
    round_results: Vec<RoundResults>,
}

fn state_monitor_bin_path(repo_dir: impl AsRef<Path>) -> PathBuf {
    repo_dir
        .as_ref()
        .join("target/release")
        .join("aleo-setup-state-monitor")
}

// TODO: add some kind of check that all specified rounds completed successfully.
pub fn integration_test(
    options: &TestOptions,
    log_writer: &LogFileWriter,
) -> eyre::Result<TestResults> {
    tracing::info!("Running integration test with options:\n{:#?}", &options);

    let out_dir = &options.out_dir;
    create_dir_if_not_exists(out_dir)?;

    // Create the log file, and write out the options that were used to run this test.
    log_writer.set_out_file(out_dir.join("integration-test.log"))?;
    let test_config_path = out_dir.join("test_config.ron");
    fs_err::write(
        test_config_path,
        ron::ser::to_string_pretty(&options, Default::default())?,
    )?;

    // Directory to store the contributor and verifier keys.
    let keys_dir_path = create_dir_if_not_exists(out_dir.join("keys"))?;

    let coordinator_dir = options.aleo_setup_coordinator_repo.dir();
    let coordinator_bin_path = coordinator_dir
        .join("target/release")
        .join("aleo-setup-coordinator");

    let setup_dir = options.aleo_setup_repo.dir();
    let _setup_frontend_dir = options.setup_frontend_repo.dir();

    // Output directory for setup1-verifier and setup1-contributor
    // projects.
    let setup_build_output_dir = setup_dir.join("target/release");
    let contributor_bin_path = setup_build_output_dir.join("setup1-contributor");
    let view_key_bin_path = setup_build_output_dir.join("view-key");

    // Create the verifiers, generate their keys.
    let verifiers: Vec<Verifier> = (1..=options.verifiers)
        .into_iter()
        .map(|i| {
            let id = format!("verifier{}", i);
            let span = tracing::error_span!("create", verifier = %id);
            let _span_guard = span.enter();

            let view_key_path = keys_dir_path.join(format!("{}.key", id));
            generate_verifier_key(&view_key_bin_path, &view_key_path)?;

            Ok(Verifier { id, view_key_path })
        })
        .collect::<eyre::Result<Vec<Verifier>>>()?;

    // Construct the configuration for each round.
    let round_configs: Vec<RoundConfig> = options
        .rounds
        .iter()
        .enumerate()
        .map(|(round_index, round)| {
            let round_number = (round_index + 1) as u64;
            let span = tracing::error_span!("round_config", round = round_number);
            let _span_guard = span.enter();

            if round.contributor_starts.len() > round.cli_contributors as usize {
                return Err(eyre::eyre!(
                    "Invalid `contributor_starts` for round {}. Its length ({}) \
                        should not exceed the number of contributors ({}).",
                    round_number,
                    round.contributor_starts.len(),
                    round.cli_contributors,
                ));
            }

            if round.contributor_drops.len() > round.cli_contributors as usize {
                return Err(eyre::eyre!(
                    "Invalid `contributor_drops` for round {}. Its length ({}) \
                        should not exceed the number of contributors ({}).",
                    round_number,
                    round.contributor_drops.len(),
                    round.cli_contributors,
                ));
            }

            // Create the contributors, generate their keys.
            let contributors: Vec<CLIContributor> = (1..=round.cli_contributors)
                .into_iter()
                .map(|i| {
                    let id = format!("contributor{}-{}", round_number, i);
                    let span = tracing::error_span!("create", contributor = %id);
                    let _span_guard = span.enter();

                    let contributor_key_file_name = format!("{}-key.json", id);
                    let key_file = keys_dir_path.join(contributor_key_file_name);

                    let contributor_key =
                        generate_contributor_key(&contributor_bin_path, &key_file).wrap_err_with(
                            || format!("Error generating contributor {} key.", id),
                        )?;

                    Ok(CLIContributor {
                        id,
                        key_file,
                        address: contributor_key.address,
                    })
                })
                .collect::<eyre::Result<Vec<CLIContributor>>>()?;

            let contributor_drops: HashMap<ContributorRef, DropContributorConfig> = round
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

            // Create the config for running each contributor.
            let contributors = contributors
                .iter()
                .enumerate()
                .map(|(i, contributor)| {
                    // Run the `setup1-contributor`.
                    let contributor_out_dir =
                        create_dir_if_not_exists(options.out_dir.join(&contributor.id))?;
                    let drop = contributor_drops
                        .get(&contributor.as_contributor_ref())
                        .cloned();

                    // By default contributors start with RoundStart
                    // unless specified in contributor_starts
                    let start = round
                        .contributor_starts
                        .get(i)
                        .cloned()
                        .unwrap_or(ContributorStartConfig::RoundStart);

                    if let ContributorStartConfig::CeremonyStart = &start {
                        return Err(eyre::eyre!(
                            "Invalid contributor_starts for round {}. {:?} \
                                is not a valid start config for a normal contributor.",
                            round_number,
                            start
                        ));
                    }

                    Ok(CLIContributorConfig {
                        id: contributor.id.clone(),
                        contributor_ref: contributor.as_contributor_ref(),
                        contributor_bin_path: contributor_bin_path.clone(),
                        key_file_path: contributor.key_file.clone(),
                        coordinator_api_url: COORDINATOR_API_URL.to_string(),
                        out_dir: contributor_out_dir,
                        drop,
                        start,
                    })
                })
                .zip(contributors.iter())
                .map::<eyre::Result<(CLIContributor, CLIContributorConfig)>, _>(|pair| {
                    match pair.0 {
                        Ok(config) => Ok((pair.1.clone(), config)),
                        Err(error) => Err(error),
                    }
                })
                .collect::<eyre::Result<Vec<(CLIContributor, CLIContributorConfig)>>>()?;

            Ok(RoundConfig {
                round_number,
                contributors,
                contributor_drops,
                verifiers: verifiers.clone(),
            })
        })
        .collect::<eyre::Result<Vec<RoundConfig>>>()?;

    // Create the replacement contributors, generate their keys.
    let replacement_contributors: Vec<(CLIContributor, CLIContributorConfig)> = (1..=options
        .replacement_contributors)
        .into_iter()
        .map(|i| {
            let id = format!("replacement_contributor{}", i);
            let contributor_key_file_name = format!("{}-key.json", id);
            let key_file = keys_dir_path.join(contributor_key_file_name);

            let contributor_key = generate_contributor_key(&contributor_bin_path, &key_file)
                .wrap_err_with(|| format!("Error generating contributor {} key.", id))?;

            let contributor = CLIContributor {
                id: id.clone(),
                key_file,
                address: contributor_key.address,
            };

            // Run the `setup1-contributor`.
            let contributor_out_dir = create_dir_if_not_exists(options.out_dir.join(&id))?;
            let contributor_config = CLIContributorConfig {
                id,
                contributor_ref: contributor.as_contributor_ref(),
                contributor_bin_path: contributor_bin_path.clone(),
                key_file_path: contributor.key_file.clone(),
                coordinator_api_url: COORDINATOR_API_URL.to_string(),
                out_dir: contributor_out_dir,
                drop: None,
                start: ContributorStartConfig::CeremonyStart,
            };

            Ok((contributor, contributor_config))
        })
        .collect::<eyre::Result<Vec<(CLIContributor, CLIContributorConfig)>>>()?;

    let replacement_contributor_refs: Vec<ContributorRef> = replacement_contributors
        .iter()
        .map(|c| c.0.as_contributor_ref())
        .collect();

    let coordinator_config = CoordinatorConfig {
        crate_dir: coordinator_dir.to_owned(),
        setup_coordinator_bin: coordinator_bin_path,
        environment: options.environment,
        out_dir: create_dir_if_not_exists(options.out_dir.join("coordinator"))?,
        replacement_contributors: replacement_contributor_refs,
    };

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let bus: Bus<CeremonyMessage> = Bus::new(1000);
    let ceremony_tx = bus.broadcaster();
    let ceremony_rx = bus.subscribe();

    let mut process_joins: Vec<Box<dyn MultiJoinable>> = Vec::new();

    let time_limit_join = options
        .timout
        .map(|timeout| ceremony_time_limit(timeout, ceremony_rx.clone(), ceremony_tx.clone()));

    // Construct MessageWaiters which wait for specific messages
    // during the ceremony before joining.
    let coordinator_ready = MessageWaiter::spawn_expected(
        vec![CeremonyMessage::RoundWaitingForParticipants(1)],
        || Ok(()),
        ceremony_rx.clone(),
    );

    // Run the coordinator.
    let coordinator_join = run_coordinator(
        &coordinator_config,
        ceremony_tx.clone(),
        ceremony_rx.clone(),
    )?;

    process_joins.push(Box::new(coordinator_join));

    if let Some(state_monitor_options) = &options.state_monitor {
        let state_monitor_config = StateMonitorConfig {
            state_monitor_bin: state_monitor_bin_path(state_monitor_options.repo.dir()),
            transcript_dir: coordinator_config.transcript_dir(),
            out_dir: options.out_dir.clone(),
            address: state_monitor_options.address,
        };

        let state_monitor_join = run_state_monitor(
            state_monitor_config,
            ceremony_tx.clone(),
            ceremony_rx.clone(),
        )?;
        process_joins.push(Box::new(state_monitor_join));
    }

    // Wait for the coordinator and coordinator proxy to start.
    coordinator_ready
        .join()
        .wrap_err("Error while waiting for coordinator to start")?;

    tracing::info!("Coordinator started.");

    if !replacement_contributors.is_empty() {
        tracing::info!(
            "Starting {} replacement contributors.",
            replacement_contributors.len()
        );
    }

    for (_, contributor_config) in replacement_contributors {
        let contributor_join =
            run_cli_contributor(contributor_config, ceremony_tx.clone(), ceremony_rx.clone())?;
        process_joins.push(Box::new(contributor_join));
    }

    for verifier in &verifiers {
        // Run the `setup1-verifier`.
        let verifier_bin_path = setup_build_output_dir.join("setup1-verifier");
        let verifier_out_dir = create_dir_if_not_exists(options.out_dir.join(&verifier.id))?;
        let verifier_join = run_verifier(
            &verifier.id,
            verifier_bin_path,
            COORDINATOR_API_URL,
            &verifier.view_key_path,
            ceremony_tx.clone(),
            ceremony_rx.clone(),
            verifier_out_dir,
        )?;
        process_joins.push(Box::new(verifier_join));
    }

    let round_results = round_configs
        .into_iter()
        .map(|round_config| {
            test_round(
                round_config,
                &coordinator_config,
                options,
                &ceremony_tx,
                &ceremony_rx,
            )
        })
        .collect::<eyre::Result<Vec<RoundResults>>>()?;

    // Tell the other threads to shutdown, safely terminating their
    // child processes.
    ceremony_tx.broadcast(CeremonyMessage::Shutdown(ShutdownReason::TestFinished))?;

    // Wait for threads to close after being told to shut down.
    join_multiple(process_joins).expect("Error while joining monitor threads.");

    if let Some(handle) = time_limit_join {
        tracing::debug!("Waiting for time limit to join");
        if let Err(error) = handle
            .join()
            .expect("error while joining time limit thread")
        {
            tracing::error!("{:?}", error);
            return Err(error);
        }
    }

    Ok(TestResults { round_results })
}

/// Configuration for running a round of the ceremony.
pub struct RoundConfig {
    /// The number of the round in the ceremony.
    round_number: u64,
    /// A vector of contributors and their configurations. New
    /// contributor processes will be started for each of these.
    contributors: Vec<(CLIContributor, CLIContributorConfig)>,
    /// A map of contributor references to the relavent drop
    /// configuration (if the contributor needs to be dropped during
    /// this round).
    contributor_drops: HashMap<ContributorRef, DropContributorConfig>,
    /// A vector of verifiers participating in this round. It is
    /// expected that the specified verifiers are already running.
    verifiers: Vec<Verifier>,
}

/// Test an individual round of the ceremony. It is expected that the
/// coordinator, verifiers and replacement contributors are already
/// running before this function is called.
fn test_round(
    round_config: RoundConfig,
    coordinator_config: &CoordinatorConfig,
    options: &TestOptions,
    ceremony_tx: &Sender<CeremonyMessage>,
    ceremony_rx: &Receiver<CeremonyMessage>,
) -> eyre::Result<RoundResults> {
    let span = tracing::error_span!("test_round", round = round_config.round_number);
    let _span_guard = span.enter();

    let mut process_joins: Vec<Box<dyn MultiJoinable>> = Vec::new();

    // Monitor the ceremony for dropped participants
    let drops_config = MonitorDropsConfig {
        contributor_drops: round_config.contributor_drops.clone(),
    };
    let monitor_drops_join = monitor_drops(drops_config, ceremony_rx.clone(), ceremony_tx.clone());

    // Construct MessageWaiters which wait for specific messages
    // during the ceremony before joining.
    let round_started = MessageWaiter::spawn_expected(
        vec![CeremonyMessage::RoundStarted(round_config.round_number)],
        || Ok(()),
        ceremony_rx.clone(),
    );
    let round_aggregation_started = MessageWaiter::spawn_expected(
        vec![CeremonyMessage::RoundStartedAggregation(
            round_config.round_number,
        )],
        || Ok(()),
        ceremony_rx.clone(),
    );
    let round_finished = MessageWaiter::spawn_expected(
        vec![CeremonyMessage::RoundFinished(round_config.round_number)],
        || Ok(()),
        ceremony_rx.clone(),
    );
    let round_aggregated = MessageWaiter::spawn_expected(
        vec![CeremonyMessage::RoundAggregated(round_config.round_number)],
        || Ok(()),
        ceremony_rx.clone(),
    );

    // Run the contributors which are to be present at the start of
    // the round.
    let starting_contributors: Vec<CLIContributor> = round_config
        .contributors
        .iter()
        .filter(|(_contributor, contributor_config)| {
            matches!(contributor_config.start, ContributorStartConfig::RoundStart)
        })
        .map(|(contributor, contributor_config)| {
            let contributor_join = run_cli_contributor(
                contributor_config.clone(),
                ceremony_tx.clone(),
                ceremony_rx.clone(),
            )?;
            process_joins.push(Box::new(contributor_join));
            Ok(contributor.clone())
        })
        .collect::<eyre::Result<Vec<CLIContributor>>>()?;

    // Configure/set-up the contributors which will join at some later
    // point during the round.
    let mid_round_contributor_joins: Vec<Box<dyn MultiJoinable>> = round_config
        .contributors
        .iter()
        .filter_map(
            |(_contributor, contributor_config)| match &contributor_config.start {
                ContributorStartConfig::AfterRoundContributions(start_config) => {
                    let process_join = JoinLater::new();
                    let waiter_process_join = process_join.clone();
                    let waiter_ceremony_tx = ceremony_tx.clone();
                    let waiter_ceremony_rx = ceremony_rx.clone();
                    let this_contributor_config = contributor_config.clone();
                    let waiter_join: Box<dyn MultiJoinable> = Box::new(spawn_contribution_waiter(
                        start_config.after_round_contributions,
                        move || {
                            let contributor_join = run_cli_contributor(
                                this_contributor_config,
                                waiter_ceremony_tx,
                                waiter_ceremony_rx,
                            )?;
                            waiter_process_join.register(contributor_join);
                            Ok(())
                        },
                        ceremony_rx.clone(),
                    ));
                    let process_join_boxed: Box<dyn MultiJoinable> = Box::new(process_join);
                    let joins: Box<dyn MultiJoinable> =
                        Box::new(JoinMultiple::new(vec![waiter_join, process_join_boxed]));
                    Some(joins)
                }
                _ => None,
            },
        )
        .collect::<Vec<Box<dyn MultiJoinable>>>();

    let mut round_errors: Vec<eyre::Error> = Vec::new();

    let round_start_time = std::time::Instant::now();

    tracing::info!("Waiting for round to start.");

    match round_started
        .join()
        .wrap_err("Error while waiting for round to start")?
    {
        WaiterJoinCondition::Shutdown => {}
        WaiterJoinCondition::MessagesReceived => {
            tracing::info!("Round has started!");

            if let Err(error) = check_participants_in_round(
                coordinator_config,
                round_config.round_number,
                &starting_contributors,
                &round_config.verifiers,
            ) {
                ceremony_tx.broadcast(CeremonyMessage::Shutdown(ShutdownReason::Error))?;
                round_errors.push(error);
            }
        }
    }

    round_aggregation_started
        .join()
        .wrap_err("Error while waiting for round aggregation to start")?
        .on_messages_received(|| {
            tracing::info!(
                "Round contributions and verifications complete. Aggregation has started."
            )
        });

    let aggregation_start_time = std::time::Instant::now();

    let aggregation_duration = match round_aggregated
        .join()
        .wrap_err("Error while waiting for round to aggregate.")?
    {
        WaiterJoinCondition::Shutdown => None,
        WaiterJoinCondition::MessagesReceived => {
            tracing::info!("Round aggregated.");
            let aggregation_duration = aggregation_start_time.elapsed();
            tracing::info!(
                "Aggregation time: {}",
                format_duration(aggregation_duration)
            );
            Some(aggregation_duration)
        }
    };

    let total_round_duration = match round_finished
        .join()
        .wrap_err("Error while waiting for round to finish.")?
    {
        WaiterJoinCondition::Shutdown => None,
        WaiterJoinCondition::MessagesReceived => {
            tracing::info!("Round finished.");
            let total_round_duration = round_start_time.elapsed();
            tracing::info!(
                "Total round time: {}",
                format_duration(total_round_duration)
            );
            Some(total_round_duration)
        }
    };

    // Wait for threads to close after being told to shut down.
    join_multiple(process_joins).expect("Error while joining process monitor threads.");
    join_multiple(mid_round_contributor_joins)
        .expect("Error while joining mid round contributor join threads");

    tracing::debug!("Waiting for monitor_drops thread to join.");
    monitor_drops_join
        .join()
        .expect("Error while monitor drops thread")?;

    tracing::info!(
        "All contributor threads/processes joined, test round {} complete!",
        round_config.round_number
    );

    if !round_errors.is_empty() {
        tracing::error!("Round completed with errors.");
        for error in &round_errors {
            tracing::error!("{:?}", error);
        }

        return Err(round_errors
            .pop()
            .expect("expected at least one error to be present"));
    }

    let results = RoundResults {
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

//! Functions for controlling/running the aleo setup coordinator
//! rocket server.

use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};
use regex::Regex;
use serde::{Deserialize, Serialize};
use subprocess::Exec;

use crate::{
    contributor::Contributor,
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    verifier::Verifier,
    AleoPublicKey, CeremonyMessage, ContributorRef, Environment, ParticipantRef, VerifierRef,
};

/// The format of the configuration json configuration file, used with
/// the `--config` command line option for `aleo-setup-coordinator`.
#[derive(Debug, Serialize)]
struct CoordinatorTomlConfiguration {
    /// The setup we are going to run.
    setup: Environment,
    /// The public keys e.g.
    /// `aleo1hsr8czcmxxanpv6cvwct75wep5ldhd2s702zm8la47dwcxjveypqsv7689`
    /// of contributors which will act as replacements for regular
    /// contributors which get dropped during a round.
    replacement_contributors: Vec<AleoPublicKey>,

    /// The option to define if we are checking the reliability
    /// score of the contributors or not. Defaults to false
    check_reliability: bool,

    /// Defines the threshold at which we let the contributors to
    /// join the queue. For example if the threshold is 8 and
    /// the reliability is 8 or above, then the contributor is allowed
    /// to join the queue.
    reliability_threshold: u8,

    /// TODO: refactor later to use a proper address type
    listen_address: SocketAddr,

    /// TODO: is this actually in use??
    database_address: SocketAddr,
}

impl From<&CoordinatorConfig> for CoordinatorTomlConfiguration {
    fn from(config: &CoordinatorConfig) -> Self {
        let replacement_contributors = config
            .replacement_contributors
            .iter()
            .map(|c| c.address.clone())
            .collect();

        Self {
            setup: config.environment,
            replacement_contributors,
            // TODO: update this when reliability checks are implemented.
            check_reliability: false,
            reliability_threshold: 8,
            listen_address: SocketAddr::from_str("0.0.0.0:9000").unwrap(),
            database_address: SocketAddr::from_str("127.0.0.1:2000").unwrap(),
        }
    }
}

/// Configuration for the [run_coordinator()] function to run
/// `aleo-setup-coordinator` rocket server.
#[derive(Debug)]
pub struct CoordinatorConfig {
    /// The location of the `aleo-setup-coordinator` repository.
    pub crate_dir: PathBuf,
    /// The location of the `aleo-setup-coordinator` binary (including
    /// the binary name).
    pub setup_coordinator_bin: PathBuf,
    /// What environment to use while running the setup ceremony.
    pub environment: Environment,
    /// The directory where all the artifacts produced while running
    /// the coordinator will be stored (and the current working
    /// directory for the process).
    pub out_dir: PathBuf,
    /// List of replacement contributors in use for the ceremony.
    pub replacement_contributors: Vec<ContributorRef>,
}

impl CoordinatorConfig {
    /// Calculates where the directory containing the ceremony
    /// transcript is located.
    pub fn transcript_dir(&self) -> PathBuf {
        if let Environment::Development = self.environment {
            self.out_dir.join("transcript/development")
        } else {
            self.out_dir.join("transcript")
        }
    }
}

/// Run the `aleo-setup-coordinator` rocket server.
pub fn run_coordinator(
    config: &CoordinatorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    let toml_config = CoordinatorTomlConfiguration::from(config);
    let toml_config_str = toml::to_string_pretty(&toml_config)
        .wrap_err("Error while serializing coordinator toml config")?;
    let toml_config_path = config.out_dir.join("config.toml");
    std::fs::write(&toml_config_path, &toml_config_str)
        .wrap_err("Error while writing corodinator config.toml file")?;

    tracing::info!("Starting setup coordinator.");

    let exec = Exec::cmd(config.setup_coordinator_bin.canonicalize()?)
        .cwd(&config.out_dir)
        .env("RUST_LOG", "debug,hyper=warn")
        .arg("--config")
        .arg(
            toml_config_path
                .canonicalize()
                .wrap_err("cannot canonicalize toml config path")?,
        );

    let log_file_path = config.out_dir.join("coordinator.log");

    let (join, _) = run_monitor_process(
        "coordinator".to_string(),
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, ceremony_tx, _monitor_tx| {
            monitor_coordinator(stdout, ceremony_tx, &log_file_path)
        }),
    )?;

    Ok(join)
}

#[derive(Debug)]
enum CoordinatorState {
    /// The process has just started.
    ProcessStarted,
    /// The coordinator is ready and the specified round is waiting
    /// for participants before it can start.
    RoundWaitingForParticipants(u64),
    /// The specified round has started and is running.
    RoundRunning(u64),
    /// The round has completed contributions and verifications, and
    /// the coordinator is aggregating chunks.
    RoundAggregating(u64),
    /// The round has completed aggregation and is now waiting for the
    /// final report.
    RoundWaitingForFinish(u64),
    /// The round has finished. Waiting to confirm that the next round
    /// is awaiting participants.
    RoundFinished(u64),
}

/// This struct keeps track of the current state of the coordinator.
struct CoordinatorStateReporter {
    ceremony_tx: Sender<CeremonyMessage>,
    current_state: CoordinatorState,
}

lazy_static::lazy_static! {
    static ref BOOTED_RE: Regex = Regex::new(".*Coordinator has booted up.*").unwrap();
    static ref ROUND1_STARTED_RE: Regex = Regex::new(".*Advanced ceremony to round 1.*").unwrap();
    static ref ROUND1_STARTED_AGGREGATION_RE: Regex = Regex::new(".*Starting aggregation on round 1").unwrap();
    static ref ROUND1_AGGREGATED_RE: Regex = Regex::new(".*Round 1 is aggregated.*").unwrap();
    static ref ROUND1_FINISHED_RE: Regex = Regex::new(".*Round 1 is finished.*").unwrap();
    static ref DROPPED_PARTICIPANT_RE: Regex = Regex::new(".*Dropping (?P<address>aleo[a-z0-9]+)[.](?P<participant_type>contributor|verifier) from the ceremony").unwrap();
    static ref SUCCESSFUL_CONTRIBUTION_RE: Regex = Regex::new(".*((?P<address>aleo[a-z0-9]+)[.]contributor) added a contribution to chunk (?P<chunk>[0-9]+)").unwrap();
}

impl CoordinatorStateReporter {
    /// Create a new [CoordinatorStateReporter] with the state that
    /// the process has just been started.
    fn process_started(ceremony_tx: Sender<CeremonyMessage>) -> Self {
        Self {
            ceremony_tx,
            current_state: CoordinatorState::ProcessStarted,
        }
    }

    /// Check whether a participant has been dropped from the round
    /// (and broadcast this fact with [CeremonyMessage::ParticipantDropped]).
    fn check_participant_dropped(&mut self, line: &str) -> eyre::Result<()> {
        if let Some(captures) = DROPPED_PARTICIPANT_RE.captures(line) {
            let address_str = captures
                .name("address")
                .expect("expected address group to be captured")
                .as_str()
                .to_string();
            let participant_type_s = captures
                .name("participant_type")
                .expect("expected participant_type group to be captured")
                .as_str();

            let address = AleoPublicKey::from_str(&address_str)?;

            let participant = match participant_type_s {
                "contributor" => ParticipantRef::Contributor(ContributorRef { address }),
                "verifier" => ParticipantRef::Verifier(VerifierRef { address }),
                _ => {
                    return Err(eyre::eyre!(
                        "unknown participant type: {}",
                        participant_type_s
                    ))
                }
            };

            self.ceremony_tx
                .broadcast(CeremonyMessage::ParticipantDropped(participant))?;
        }

        Ok(())
    }

    /// Parse stdout line from the `coordinator` process, broadcast
    /// messages to the ceremony when the coordinator state changes.
    /// Keeps track of the current state of the ceremony.
    fn parse_output_line(&mut self, line: &str) -> eyre::Result<()> {
        match self.current_state {
            CoordinatorState::ProcessStarted => {
                if BOOTED_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundWaitingForParticipants(1))?;
                    self.current_state = CoordinatorState::RoundWaitingForParticipants(1);
                }
            }
            CoordinatorState::RoundWaitingForParticipants(1) => {
                self.check_participant_dropped(line)?;
                if ROUND1_STARTED_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundStarted(1))?;
                    self.current_state = CoordinatorState::RoundRunning(1);
                }
            }
            CoordinatorState::RoundRunning(1) => {
                self.check_participant_dropped(line)?;
                if ROUND1_STARTED_AGGREGATION_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundStartedAggregation(1))?;
                    self.current_state = CoordinatorState::RoundAggregating(1);
                }

                if let Some(captures) = SUCCESSFUL_CONTRIBUTION_RE.captures(line) {
                    let address_str = captures
                        .name("address")
                        .expect("expected address group to be captured")
                        .as_str()
                        .to_string();

                    let chunk = u64::from_str(
                        captures
                            .name("chunk")
                            .expect("exprected chunk address to be captured")
                            .as_str(),
                    )?;

                    let address = AleoPublicKey::from_str(&address_str)?;

                    tracing::debug!(
                        "Contributor {} made a successful contribution to chunk {}.",
                        &address,
                        &chunk
                    );

                    let contributor = ContributorRef { address };
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::SuccessfulContribution {
                            contributor,
                            chunk,
                        })?;
                }
            }
            CoordinatorState::RoundAggregating(1) => {
                if ROUND1_AGGREGATED_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundAggregated(1))?;
                    self.current_state = CoordinatorState::RoundWaitingForFinish(1);
                }
            }
            CoordinatorState::RoundWaitingForFinish(1) => {
                if ROUND1_FINISHED_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundFinished(1))?;
                    self.current_state = CoordinatorState::RoundFinished(1);
                }
            }
            CoordinatorState::RoundFinished(1) => {
                // TODO Multiple rounds are not yet supported.
                return Ok(());
            }
            _ => return Err(eyre::eyre!("unhandled state: {:?}", self.current_state)),
        }

        Ok(())
    }
}

/// Monitor the setup coordinator. Parses the `stderr`/`stdout` and
/// emits messages/alters state when certain events occur, and also
/// pipes the output to the [tracing::debug!()], and
/// `coordinator_log.txt` log file.
fn monitor_coordinator(
    stdout: File,
    ceremony_tx: Sender<CeremonyMessage>,
    log_file_path: impl AsRef<Path>,
) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);
    let mut state_reporter = CoordinatorStateReporter::process_started(ceremony_tx);

    let mut log_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_file_path)
        .wrap_err("unable to open log file")?;

    // It's expected that if the process closes, the stdout will also
    // close and this iterator will complete gracefully.
    for line_result in buf_pipe.lines() {
        match line_result {
            Ok(line) => {
                state_reporter.parse_output_line(&line)?;

                // Write to log file.
                log_file.write(line.as_ref())?;
                log_file.write("\n".as_ref())?;
            }
            Err(error) => {
                tracing::error!(
                    "Error reading line from pipe to coordinator process: {}",
                    error
                )
            }
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct RoundState {
    /// The ids of the contributors in the round.
    #[serde(rename = "contributorIds")]
    contributor_ids: Vec<String>,
    /// The ids of the verifiers in the round.
    #[serde(rename = "verifierIds")]
    verifier_ids: Vec<String>,
}

/// Check that the specified participants are in the specified round
/// transcript.
pub fn check_participants_in_round(
    config: &CoordinatorConfig,
    round: u64,
    contributors: &[Contributor],
    verifiers: &[Verifier],
) -> eyre::Result<()> {
    let state_file = config
        .transcript_dir()
        .join(format!("round_{}", round))
        .join("state.json");

    let state_file_str = std::fs::read_to_string(&state_file)
        .wrap_err_with(|| eyre::eyre!("Unable to read state file: {:?}", &state_file))?;

    let state: RoundState = serde_json::from_str(&state_file_str)
        .wrap_err_with(|| eyre::eyre!("Unable to deserialize state file: {:?}", state_file))?;

    for contributor in contributors {
        state
            .contributor_ids
            .iter()
            .find(|round_contributor_id| round_contributor_id == &&contributor.id_on_coordinator())
            .ok_or_else(|| {
                eyre::eyre!(
                    "Unable to find contributor {} in round state file",
                    contributor.id_on_coordinator()
                )
            })?;
    }

    // TODO: use the same logic as checking contributors, when I can
    // calculate the verifier public key/coordinator id.
    if verifiers.len() != state.verifier_ids.len() {
        return Err(eyre::eyre!(
            "Number of verifiers in the round {}, does not match \
                the number of verifiers started for the round: {}",
            state.verifier_ids.len(),
            verifiers.len()
        ));
    }

    Ok(())
}

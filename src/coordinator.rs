//! Functions for controlling/running the aleo setup coordinator
//! rocket server.

use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};
use regex::Regex;
use subprocess::Exec;

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage, Environment,
};

/// Copy the `Rocket.toml` config file from the
/// `aleo-setup-cooridinator` repository to the out dir, which is the
/// current working directory while running the coordinator.
pub fn deploy_coordinator_rocket_config(config: &CoordinatorConfig) -> eyre::Result<()> {
    let config_path = config.crate_dir.join("Rocket.toml");
    let config_deploy_path = config.out_dir.join("Rocket.toml");

    std::fs::copy(config_path, config_deploy_path)
        .wrap_err("Error while deploying coordinator Rocket.toml config file")
        .map(|_| ())
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
}

pub struct CoordinatorRunDetails {
    pub join: MonitorProcessJoin,
    pub transcript_dir: PathBuf,
}

/// Run the `aleo-setup-coordinator` rocket server.
pub fn run_coordinator(
    config: &CoordinatorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<CoordinatorRunDetails> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Starting setup coordinator.");

    let exec = Exec::cmd(config.setup_coordinator_bin.canonicalize()?)
        .cwd(&config.out_dir)
        .arg("--setup")
        .arg(config.environment.to_string());

    let log_file_path = config.out_dir.join("coordinator.log");

    let join = run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, ceremony_tx| {
            monitor_coordinator(stdout, ceremony_tx, &log_file_path)
        }),
    )?;

    let transcript_dir = match config.environment {
        Environment::Development => config.out_dir.join("transcript/development"),
        _ => config.out_dir.join("transcript"),
    };

    Ok(CoordinatorRunDetails {
        join,
        transcript_dir,
    })
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
    static ref ROCKET_LAUNCH_RE: Regex = Regex::new("Rocket has launched.*").unwrap();
    static ref ROUND1_STARTED_RE: Regex = Regex::new(".*Advanced ceremony to round 1.*").unwrap();
    static ref ROUND1_STARTED_AGGREGATION_RE: Regex = Regex::new(".*Starting aggregation on round 1").unwrap();
    static ref ROUND1_AGGREGATED_RE: Regex = Regex::new(".*Round 1 is aggregated.*").unwrap();
    static ref ROUND1_FINISHED_RE: Regex = Regex::new(".*Round 1 is finished.*").unwrap();
}

impl CoordinatorStateReporter {
    /// Create a new [CoordinatorStateReporter] with the state that
    /// the process has just been started.
    pub fn process_started(ceremony_tx: Sender<CeremonyMessage>) -> Self {
        Self {
            ceremony_tx,
            current_state: CoordinatorState::ProcessStarted,
        }
    }

    /// Parse stdout line from the `coordinator` process, broadcast
    /// messages to the ceremony when the coordinator state changes.
    /// Keeps track of the current state of the ceremony.
    pub fn parse_output_line(&mut self, line: &str) -> eyre::Result<()> {
        match self.current_state {
            CoordinatorState::ProcessStarted => {
                if ROCKET_LAUNCH_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundWaitingForParticipants(1))?;
                    self.current_state = CoordinatorState::RoundWaitingForParticipants(1);
                }
            }
            CoordinatorState::RoundWaitingForParticipants(1) => {
                if ROUND1_STARTED_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundStarted(1))?;
                    self.current_state = CoordinatorState::RoundRunning(1);
                }
            }
            CoordinatorState::RoundRunning(1) => {
                if ROUND1_STARTED_AGGREGATION_RE.is_match(&line) {
                    self.ceremony_tx
                        .broadcast(CeremonyMessage::RoundStartedAggregation(1))?;
                    self.current_state = CoordinatorState::RoundAggregating(1);
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
                // TODO: multiple rounds are not yet supported.
            }
            _ => return Err(eyre::eyre!("unhandled state: {:?}", self.current_state)),
        }

        Ok(())
    }
}

/// Monitor the setup coordinator. Watches for the `Rocket has
/// launched` message, which when it occurs emits a
/// [CeremonyMessage::CoordinatorReady] message. Pipes the
/// `stderr`/`stdout` to the [tracing::debug!()], and
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

                // Pipe the process output to tracing.
                tracing::debug!("{}", line);

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

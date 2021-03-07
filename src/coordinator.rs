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
    CeremonyMessage, SetupPhase,
};

/// Configuration for the [run_coordinator()] function to run
/// `aleo-setup-coordinator` rocket server.
#[derive(Debug)]
pub struct CoordinatorConfig {
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
pub fn run_coordinator(
    config: CoordinatorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    log_dir_path: PathBuf,
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Starting setup coordinator.");

    let exec = Exec::cmd(std::fs::canonicalize(config.setup_coordinator_bin)?)
        .cwd(config.crate_dir)
        .arg(config.phase.to_string());

    let log_file_path = log_dir_path.join("coordinator.log");
    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, ceremony_tx| {
            monitor_coordinator(stdout, ceremony_tx, &log_file_path)
        }),
    )
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

    let start_re = Regex::new("Rocket has launched.*")?;
    let round1_started_re = Regex::new(".*Advanced ceremony to round 1.*")?;
    let round1_finished_re = Regex::new(".*Round 1 is finished.*")?;
    let round1_aggregated_re = Regex::new(".*Round 1 is aggregated.*")?;

    let mut started = false;
    let mut round1_started = false;
    let mut round1_finished = false;
    let mut round1_aggregated = false;

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
                // TODO: refactor this logic into a state machine to
                // ensure that the round cannot be detected as
                // finished before it has started.
                if !started && start_re.is_match(&line) {
                    ceremony_tx.broadcast(CeremonyMessage::CoordinatorReady)?;
                    started = true;
                }

                if !round1_started && round1_started_re.is_match(&line) {
                    ceremony_tx.broadcast(CeremonyMessage::RoundStarted(1))?;
                    round1_started = true;
                }

                if !round1_finished && round1_finished_re.is_match(&line) {
                    ceremony_tx.broadcast(CeremonyMessage::RoundFinished(1))?;
                    round1_finished = true;
                }

                if !round1_aggregated && round1_aggregated_re.is_match(&line) {
                    ceremony_tx.broadcast(CeremonyMessage::RoundAggregated(1))?;
                    round1_aggregated = true;
                }

                // Pipe the process output to tracing.
                tracing::debug!("{}", line);

                // Write to log file.
                log_file.write(line.as_ref())?;
                log_file.write("\n".as_ref())?;
            }
            Err(error) => {
                tracing::error!("Error reading line from pipe to nodejs process: {}", error)
            }
        }
    }

    Ok(())
}

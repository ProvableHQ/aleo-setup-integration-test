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
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Starting setup coordinator.");

    let exec = Exec::cmd(std::fs::canonicalize(config.setup_coordinator_bin)?)
        .cwd(config.crate_dir)
        .arg(config.phase.to_string());

    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(monitor_coordinator),
    )
}

/// Monitor the setup coordinator. Watches for the `Rocket has
/// launched` message, which when it occurs emits a
/// [CeremonyMessage::CoordinatorReady] message. Pipes the
/// `stderr`/`stdout` to the [tracing::debug!()], and
/// `coordinator_log.txt` log file.
fn monitor_coordinator(stdout: File, ceremony_tx: Sender<CeremonyMessage>) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

    let start_re = Regex::new("Rocket has launched.*")?;

    let log_path = Path::new("coordinator_log.txt");
    let current_dir = std::env::current_dir()?;
    let mut log_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_path)
        .wrap_err_with(|| {
            format!(
                "Unable to open log file {:?} in {:?}",
                log_path, current_dir
            )
        })?;

    // It's expected that if the process closes, the stdout will also
    // close and this iterator will complete gracefully.
    for line_result in buf_pipe.lines() {
        match line_result {
            Ok(line) => {
                if start_re.is_match(&line) {
                    ceremony_tx.broadcast(CeremonyMessage::CoordinatorReady)?;
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

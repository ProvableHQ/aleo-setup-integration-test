use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use eyre::Context;
use flume::{Receiver, Sender};
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
///
/// TODO: return a thread join handle.
/// TODO: make a monitor thread (like in the proxy).
pub fn run_coordinator(
    config: CoordinatorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Setup coordinator waiting for nodejs proxy to start.");

    // Wait for the coordinator proxy to report that it's ready.
    for message in ceremony_rx.recv() {
        match message {
            CeremonyMessage::CoordinatorProxyReady => break,
            CeremonyMessage::Shutdown => {
                return Err(eyre::eyre!(
                    "Ceremony shutdown before coordinator could start."
                ))
            }
            _ => {
                tracing::error!("Unexpected message: {:?}", message);
            }
        }
    }

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

fn monitor_coordinator(stdout: File, ceremony_tx: Sender<CeremonyMessage>) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

    let start_re = Regex::new("Rocket has launched.*")?;

    let log_path = Path::new("coordinator_log.txt");
    let current_dir = std::env::current_dir()?;
    let log_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_path)
        .wrap_err_with(|| {
            format!(
                "Unable to open log file {:?} in {:?}",
                log_path, current_dir
            )
        })?;

    let mut buf_log = BufWriter::new(log_file);

    // It's expected that if the process closes, the stdout will also
    // close and this iterator will complete gracefully.
    for line_result in buf_pipe.lines() {
        match line_result {
            Ok(line) => {
                if start_re.is_match(&line) {
                    ceremony_tx.send(CeremonyMessage::CoordinatorReady)?;
                }

                // Pipe the process output to tracing.
                tracing::debug!("{}", line);

                // Write to log file.
                buf_log.write(line.as_ref())?;
                buf_log.write("\n".as_ref())?;
            }
            Err(error) => {
                tracing::error!("Error reading line from pipe to nodejs process: {}", error)
            }
        }
    }

    Ok(())
}

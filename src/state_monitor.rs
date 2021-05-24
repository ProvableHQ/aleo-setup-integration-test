//! Functions for running/controlling the `aleo-setup-state-monitor` server.

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};

use subprocess::Exec;

use std::{
    fmt::Debug,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
};

/// Install the dependencies for running the state monitor.
pub fn setup_state_monitor(state_monitor_dir: impl AsRef<Path> + Debug) -> eyre::Result<()> {
    Exec::cmd("bash")
        .cwd(state_monitor_dir)
        .arg("install_serve.sh")
        .join()
        .map_err(eyre::Error::from)
        .map_err(|error| error.wrap_err(format!("Error running `install_serve.sh`")))
        .and_then(default_parse_exit_status)
}

/// Starts the `aleo-setup-state-monitor` server.
pub fn run_state_monitor(
    state_monitor_dir: impl AsRef<Path> + Debug,
    transcript_dir: impl AsRef<Path> + Debug,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    out_dir: impl AsRef<Path>,
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("state_monitor");
    let _guard = span.enter();

    tracing::info!("Starting setup state monitor.");

    if !state_monitor_dir.as_ref().exists() {
        return Err(eyre::eyre!(
            "Supplied state_monitor_repo {:?} does not exist.",
            state_monitor_dir
        ));
    }

    let exec = Exec::cmd("bash")
        .env("TRANSCRIPT_DIR", transcript_dir.as_ref())
        .arg(state_monitor_dir.as_ref().join("start_serve.sh"));

    let log_file_path = out_dir.as_ref().join("state_monitor.log");

    let (join, _) = run_monitor_process(
        "state_monitor".to_string(),
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, ceremony_tx, _monitor_tx| {
            monitor_state_monitor(stdout, ceremony_tx, &log_file_path)
        }),
    )?;

    tracing::info!("Running setup state monitor on http://127.0.0.1:5001");

    Ok(join)
}

/// Monitor the `aleo-setup-state-monitor`.
fn monitor_state_monitor(
    stdout: File,
    _ceremony_tx: Sender<CeremonyMessage>,
    log_file_path: impl AsRef<Path>,
) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

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
                // Write to log file.
                log_file.write(line.as_ref())?;
                log_file.write("\n".as_ref())?;
            }
            Err(error) => {
                tracing::error!(
                    "Error reading line from pipe to aleo setup state monitor process: {}",
                    error
                )
            }
        }
    }

    Ok(())
}

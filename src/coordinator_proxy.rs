use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};
use regex::Regex;
use subprocess::Exec;

use std::{
    fmt::Debug,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

/// Starts the nodejs proxy for the setup coordinator server.
///
/// Currently this doesn't cleanly shut down, there doesn't appear to
/// be an easy way to share the process between the line reader, and
/// the coordinator message listener.
pub fn run_coordinator_proxy(
    setup_coordinator_repo: impl AsRef<Path> + Debug,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    log_dir_path: PathBuf,
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("coordinator_proxy");
    let _guard = span.enter();

    tracing::info!("Starting setup coordinator nodejs proxy.");

    let exec = Exec::cmd("node")
        .cwd(setup_coordinator_repo)
        .arg("server.js");

    let log_file_path = log_dir_path.join("coordinator_proxy_log.txt");

    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, ceremony_tx| {
            setup_coordinator_proxy_monitor(stdout, ceremony_tx, &log_file_path)
        }),
    )
}

/// This function reads stdout from the setup coordinator nodejs proxy
/// process, and analyzes the output line by line searching for the
/// `Websocket listening is on.` message, and notifies the
/// `coordinator_rx` listeners that the proxy is ready. Also this
/// pipes the stdout from the nodejs proxy to [tracing::debug!()]
fn setup_coordinator_proxy_monitor(
    stdout: File,
    ceremony_tx: Sender<CeremonyMessage>,
    log_file_path: impl AsRef<Path>,
) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

    let start_re = Regex::new("Websocket listening on.*")?;

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
                if start_re.is_match(&line) {
                    ceremony_tx.broadcast(CeremonyMessage::CoordinatorProxyReady)?;
                }

                // Pipe the process output to tracing.
                tracing::debug!("{}", line);

                // Write to log file.
                log_file.write(line.as_ref())?;
                log_file.write("\n".as_ref())?;
            }
            Err(error) => tracing::error!(
                "Error reading line from pipe to coordinator process: {}",
                error
            ),
        }
    }

    Ok(())
}

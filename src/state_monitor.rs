//! Functions for running/controlling the `aleo-setup-state-monitor` server.

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    specification::LaunchBrowser,
    CeremonyMessage,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};

use subprocess::Exec;
use url::Url;

use std::{
    convert::TryInto,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

pub struct StateMonitorConfig {
    /// Path to the state monitor binary.
    pub state_monitor_bin: PathBuf,
    /// Directory where the ceremonytranscript is located.
    pub transcript_dir: PathBuf,
    /// Address to use for the state monitor web server.
    pub address: SocketAddr,
    /// The directory where all the artifacts produced while running
    /// the state monitor will be stored (and the current working
    /// directory for the process).
    pub out_dir: PathBuf,
    /// If `Some`, the address for the state monitor will be opened using a web browser.
    pub launch_browser: Option<LaunchBrowser>,
}

/// Starts the `aleo-setup-state-monitor` server.
pub fn run_state_monitor(
    config: StateMonitorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<MonitorProcessJoin> {
    let span = tracing::error_span!("state_monitor");
    let _guard = span.enter();

    tracing::info!("Starting setup state monitor.");

    if !config.state_monitor_bin.exists() {
        return Err(eyre::eyre!(
            "State monitor binary {:?} does not exist.",
            config.state_monitor_bin
        ));
    }

    let exec = Exec::cmd(config.state_monitor_bin.canonicalize()?)
        .arg("--transcript")
        .arg(config.transcript_dir)
        .arg("--address")
        .arg(config.address.to_string());

    let log_file_path = config.out_dir.join("state_monitor.log");

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

    let url = Url::parse(&format!("http://{}", config.address))?;

    tracing::info!("Running setup state monitor on {}", &url);

    if let Some(launch_browser) = config.launch_browser {
        if let Err(error) = webbrowser::open_browser(launch_browser.try_into()?, url.as_ref())
            .wrap_err("Error while automatically launching browser")
        {
            tracing::error!("{}", error)
        }
    }

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
                log_file.write_all(line.as_ref())?;
                log_file.write_all("\n".as_ref())?;
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

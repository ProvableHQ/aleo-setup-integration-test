//! Module containing functions for controlling/running a
//! `setup1-verifier` ceremony verifier.

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage, Environment,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};

use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

pub struct VerifierViewKey(String);

impl AsRef<str> for VerifierViewKey {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl std::fmt::Display for VerifierViewKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Use `setup1-contributor` to generate the contributor key file used
/// in [run_contributor()].
pub fn generate_verifier_key(
    view_key_bin_path: impl AsRef<Path> + std::fmt::Debug,
) -> eyre::Result<VerifierViewKey> {
    tracing::info!("Generating verifier view key.");

    let capture = subprocess::Exec::cmd(view_key_bin_path.as_ref())
        .capture()
        .map_err(eyre::Error::from)?;

    default_parse_exit_status(capture.exit_status)?;

    let view_key_out = capture.stdout_str();
    let view_key = view_key_out
        .split("\n")
        .next()
        .expect("Expected to be able to split view key output with \\n");

    assert!(!view_key.is_empty());
    tracing::info!("Generated view key: {}", view_key);

    Ok(VerifierViewKey(view_key.to_string()))
}

/// Run the `setup1-verifier`.
pub fn run_verifier(
    id: &str,
    verifier_bin_path: impl AsRef<Path>,
    environment: Environment,
    coordinator_api_url: &str,
    view_key: &VerifierViewKey,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    out_dir: PathBuf,
) -> eyre::Result<MonitorProcessJoin> {
    let view_key: &str = view_key.as_ref();
    let span = tracing::error_span!("verifier", id = id, view_key = view_key);
    let _guard = span.enter();

    tracing::info!("Running verifier.");

    let exec = subprocess::Exec::cmd(verifier_bin_path.as_ref().canonicalize()?)
        .cwd(&out_dir)
        .arg(format!("{}", environment)) // <ENVIRONMENT>
        .arg(coordinator_api_url) // <COORDINATOR_API_URL>
        .arg(view_key) // <VERIFIER_VIEW_KEY>
        .arg("DEBUG"); // log level

    let log_file_path = out_dir.join("verifier.log");

    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, _ceremony_tx| verifier_monitor(stdout, &log_file_path)),
    )
    .wrap_err_with(|| format!("Error running verifier {:?}", verifier_bin_path.as_ref()))
}

/// Monitors the `setup1-contributor`, logs output to `log_file_path`
/// file and `tracing::debug!()`.
fn verifier_monitor(stdout: File, log_file_path: impl AsRef<Path>) -> eyre::Result<()> {
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

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage, SetupPhase,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};

use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

const VERIFIER_VIEW_KEY: &str = "AViewKey1qSVA1womAfkkGHzBxeptAz781b9stjaTj9fFnEU2TC47";

/// Run the `setup1-verifier`.
pub fn run_verifier<PB>(
    verifier_bin_path: PB,
    setup_phase: SetupPhase,
    coordinator_api_url: String,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    log_dir_path: PathBuf,
) -> eyre::Result<MonitorProcessJoin>
where
    PB: AsRef<OsStr> + std::fmt::Debug,
{
    let span = tracing::error_span!("verifier");
    let _guard = span.enter();

    tracing::info!("Running verifier.");

    let exec = subprocess::Exec::cmd(&verifier_bin_path)
        .arg(format!("{}", setup_phase)) // <ENVIRONMENT>
        .arg(coordinator_api_url) // <COORDINATOR_API_URL>
        .arg(VERIFIER_VIEW_KEY) // <VERIFIER_VIEW_KEY>
        .arg("DEBUG"); // log level

    let log_file_path = log_dir_path.join("verifier.log");

    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, _ceremony_tx| verifier_monitor(stdout, &log_file_path)),
    )
    .wrap_err_with(|| format!("Error running verifier {:?}", verifier_bin_path))
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

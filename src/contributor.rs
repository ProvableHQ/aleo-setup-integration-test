//! Functions for controlling/running a `setup1-contributor` ceremony
//! contributor.

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage, SetupPhase,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};
use serde::Deserialize;

use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
pub struct ContributorKey {
    #[serde(rename = "encryptedSeed")]
    pub encrypted_seed: String,
    /// Aleo address
    pub address: String,
}

/// Use `setup1-contributor` to generate the contributor key file used
/// in [run_contributor()].
pub fn generate_contributor_key<PB, PK>(
    contributor_bin_path: PB,
    key_file_path: PK,
) -> eyre::Result<ContributorKey>
where
    PB: AsRef<OsStr> + std::fmt::Debug,
    PK: AsRef<OsStr>,
{
    tracing::info!("Generating contributor key.");

    subprocess::Exec::cmd(contributor_bin_path)
        .arg("generate")
        .args(&["--passphrase", "test"]) // <COORDINATOR_API_URL>
        .arg(key_file_path.as_ref()) // <KEYS_PATH>
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)?;

    let key_file = File::open(Path::new(&key_file_path))?;
    let contributor_key: ContributorKey = serde_json::from_reader(key_file)?;
    Ok(contributor_key)
}

/// Run the `setup1-contributor`.
pub fn run_contributor(
    contributor_bin_path: PathBuf,
    key_file_path: PathBuf,
    setup_phase: SetupPhase,
    coordinator_api_url: String,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    out_dir_path: PathBuf,
) -> eyre::Result<MonitorProcessJoin> {
    let key_file = File::open(Path::new(&key_file_path))?;
    let contributor_key: ContributorKey = serde_json::from_reader(key_file)?;

    let span = tracing::error_span!("contributor", address = contributor_key.address.as_str());
    let _guard = span.enter();

    tracing::info!("Running contributor.");

    let exec = subprocess::Exec::cmd(contributor_bin_path.canonicalize()?)
        .cwd(&out_dir_path)
        .arg("contribute")
        .args(&["--passphrase", "test"])
        .arg(format!("{}", setup_phase)) // <ENVIRONMENT>
        .arg(coordinator_api_url) // <COORDINATOR_API_URL>
        .arg(key_file_path.canonicalize()?);

    let log_file_path = out_dir_path.join("contributor.log");

    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(move |stdout, _ceremony_tx| contributor_monitor(stdout, &log_file_path)),
    )
}

/// Monitors the `setup1-contributor`, logs output to `log_file_path`
/// file and `tracing::debug!()`.
fn contributor_monitor(stdout: File, log_file_path: impl AsRef<Path>) -> eyre::Result<()> {
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

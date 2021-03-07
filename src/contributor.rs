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
    path::Path,
};

#[derive(Deserialize)]
pub struct ContributorKey {
    #[serde(rename = "encryptedSeed")]
    pub encrypted_seed: String,
    /// Aleo address
    pub address: String,
}

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
pub fn run_contributor<PB, PK>(
    contributor_bin_path: PB,
    key_file_path: PK,
    setup_phase: SetupPhase,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<MonitorProcessJoin>
where
    PB: AsRef<OsStr> + std::fmt::Debug,
    PK: AsRef<OsStr>,
{
    let key_file = File::open(Path::new(&key_file_path))?;
    let contributor_key: ContributorKey = serde_json::from_reader(key_file)?;

    let span = tracing::error_span!("contributor", address = contributor_key.address.as_str());
    let _guard = span.enter();

    tracing::info!("Running contributor.");

    let exec = subprocess::Exec::cmd(contributor_bin_path)
        .arg("contribute")
        .args(&["--passphrase", "test"])
        .arg(format!("{}", setup_phase)) // <ENVIRONMENT>
        .arg("http://localhost:9000") // <COORDINATOR_API_URL>
        .arg(key_file_path);

    run_monitor_process(
        exec,
        default_parse_exit_status,
        ceremony_tx,
        ceremony_rx,
        fallible_monitor(contributor_monitor),
    )
}

fn contributor_monitor(stdout: File, _ceremony_tx: Sender<CeremonyMessage>) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

    let log_path = Path::new("contributor_log.txt");
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
                // Pipe the process output to tracing.
                tracing::info!("{}", line);

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

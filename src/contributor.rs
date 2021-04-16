//! Functions for controlling/running a `setup1-contributor` ceremony
//! contributor.

use crate::{
    process::{
        default_parse_exit_status, fallible_monitor, run_monitor_process, MonitorProcessJoin,
    },
    CeremonyMessage, ContributorRef, Environment,
};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};
use serde::Deserialize;

use std::{
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
pub fn generate_contributor_key(
    contributor_bin_path: impl AsRef<Path> + std::fmt::Debug,
    key_file_path: impl AsRef<Path>,
) -> eyre::Result<ContributorKey> {
    tracing::info!("Generating contributor key.");

    subprocess::Exec::cmd(contributor_bin_path.as_ref())
        .arg("generate")
        .args(&["--passphrase", "test"]) // <COORDINATOR_API_URL>
        .arg(key_file_path.as_ref()) // <KEYS_PATH>
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)?;

    let key_file = File::open(key_file_path)?;
    let contributor_key: ContributorKey = serde_json::from_reader(key_file)?;
    Ok(contributor_key)
}
/// Data relating to a contributor.
#[derive(Clone)]
pub struct Contributor {
    /// A short id used to reference this contributor with the
    /// integration test. See [Contributor::coordinator_id()] for the id
    /// that the coordinator uses to refer to the contributor.
    pub id: String,
    pub key_file: PathBuf,
    /// Aleo address
    /// e.g. `aleo18whcjapew3smcwnj9lzk29vdhpthzank269vd2ne24k0l9dduqpqfjqlda`
    pub address: String,
}

impl Contributor {
    /// The id used to reference this contributor by the coordinator,
    /// and within the ceremony transcript.
    pub fn id_on_coordinator(&self) -> String {
        format!("{}.contributor", self.address)
    }

    pub fn as_contributor_ref(&self) -> ContributorRef {
        ContributorRef {
            address: self.address.clone(),
        }
    }
}

/// Configuration for running a contributor.
#[derive(Debug)]
pub struct ContributorConfig {
    /// An identifier for this contributor, also used as the name of
    /// the working directory for this contributor.
    pub id: String,
    /// The path to the binary to run this contributor,
    pub contributor_bin_path: PathBuf,
    /// The path to the key file used by this contributor.
    pub key_file_path: PathBuf,
    /// What type of ceremony will be performed.
    pub environment: Environment,
    /// The url to connect to the coordinator.
    pub coordinator_api_url: String,
    /// The out directory for the ceremony, the working directory for
    /// this contributor is `out_dir`/`id`.
    pub out_dir: PathBuf,
}

/// Run the `setup1-contributor`.
pub fn run_contributor(
    config: ContributorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<MonitorProcessJoin> {
    let key_file = File::open(&config.key_file_path)?;
    let contributor_key: ContributorKey = serde_json::from_reader(key_file)?;

    let span = tracing::error_span!(
        "contributor",
        id = %config.id,
        address = %contributor_key.address
    );
    let _guard = span.enter();

    tracing::info!("Running contributor.");

    let exec = subprocess::Exec::cmd(&config.contributor_bin_path.canonicalize()?)
        .cwd(&config.out_dir)
        .env("RUST_LOG", "debug,hyper=warn")
        .arg("contribute")
        .args(&["--passphrase", "test"])
        .arg(format!("{}", &config.environment)) // <ENVIRONMENT>
        .arg(&config.coordinator_api_url) // <COORDINATOR_API_URL>
        .arg(config.key_file_path.canonicalize()?);

    let log_file_path = config.out_dir.join("contributor.log");

    run_monitor_process(
        config.id.to_string(),
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

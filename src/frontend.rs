//! Module to handle running the host for the browser contributor's frontend code using `npm`.

use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

use eyre::Context;
use fs_err::OpenOptions;
use mpmc_bus::{Receiver, Sender};
use regex::Regex;
use subprocess::{Popen, Redirection};
use url::Url;

pub struct FrontendConfiguration {
    /// Path to the setup frontend repository.
    pub frontend_repo_dir: PathBuf,
    /// Url to use to connect to the backend.
    pub backend_url: Url,
    /// Path to directory where
    pub out_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum FrontendServerControlMessage {
    /// Tell the frontend server to shutdown.
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum FrontendServerStatusMessage {
    /// The frontend server has started.
    Started,
    /// The frontend server has shutdown.
    Shutdown,
}

/// Start the frontend development server.
pub fn start_frontend_dev_server(
    config: FrontendConfiguration,
) -> eyre::Result<(
    Sender<FrontendServerControlMessage>,
    Receiver<FrontendServerStatusMessage>,
)> {
    let control_bus = mpmc_bus::Bus::new(100);
    let status_bus = mpmc_bus::Bus::new(100);

    let backend_url_string: String = config
        .backend_url
        .to_string()
        .strip_suffix("/")
        .ok_or_else(|| {
            eyre::eyre!("Expected backend_url.to_string() to end with \"/\" forward slash")
        })?
        .to_string();

    tracing::info!(
        "Starting frontend server with REACT_APP_CEREMONY_URL={}",
        &backend_url_string
    );

    let mut process = subprocess::Exec::cmd("npm")
        .arg("start")
        .cwd(&config.frontend_repo_dir)
        .env("REACT_APP_CEREMONY_URL", backend_url_string)
        // This disables launching of the browser at startup.
        .env("BROWSER", "none")
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Merge)
        .popen()
        .wrap_err("Error opening process")?;

    // Extract the stdout [std::fs::File] from `process`, replacing it
    // with a None. This is needed so we can both listen to stdout and
    // interact with `process`'s mutable methods (to terminate it if
    // required).
    let mut stdout: Option<File> = None;
    std::mem::swap(&mut process.stdout, &mut stdout);
    let stdout = stdout.ok_or_else(|| eyre::eyre!("Unable to obtain process `stdout`."))?;

    let mut control_rx = control_bus.subscribe();
    std::thread::spawn(move || {
        let span = tracing::error_span!("frontend_server_control");
        let _guard = span.enter();

        fn process_message(
            process: &mut Popen,
            message: FrontendServerControlMessage,
        ) -> eyre::Result<()> {
            match message {
                FrontendServerControlMessage::Shutdown => {
                    process.terminate()?;
                    process.wait()?;
                    Ok(())
                }
            }
        }

        loop {
            match control_rx.recv() {
                Ok(message) => {
                    if let Err(error) = process_message(&mut process, message) {
                        tracing::error!("{}", error);
                    }
                }
                Err(error) => tracing::error!("{}", error),
            }
        }
    });

    let status_tx = status_bus.broadcaster();
    let log_file_path = config.out_dir.join("frontend_server.log");
    std::thread::spawn::<_, eyre::Result<()>>(move || {
        let span = tracing::error_span!("frontend_server_monitor");
        let _guard = span.enter();

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

                    if let Some(message) = parse_output_line(&line)? {
                        status_tx.broadcast(message)?;
                    }
                }
                Err(error) => tracing::error!(
                    "Error reading line from pipe to coordinator process: {}",
                    error
                ),
            }
        }

        Ok(())
    });

    tracing::info!("Waiting for frontend server to start.");

    let mut wait_status_rx = status_bus.subscribe();
    loop {
        match wait_status_rx.recv()? {
            FrontendServerStatusMessage::Started => break,
            FrontendServerStatusMessage::Shutdown => break,
        }
    }

    tracing::info!("Frontend server started.");

    Ok((control_bus.broadcaster(), status_bus.subscribe()))
}

lazy_static::lazy_static! {
    /// This message occurs at roughly the same time as when the frontend development server has
    /// started.
    /// TODO: this message could also perhaps be: "Files successfully emitted, waiting for
    /// typecheck results...", the output seems to change from time to time, perhaps depending on
    /// whether it's a clean compile or using something cached.
    static ref STARTED_RE: Regex = Regex::new(".*Compiled with warnings.*").unwrap();
}

fn parse_output_line(line: &str) -> eyre::Result<Option<FrontendServerStatusMessage>> {
    if STARTED_RE.is_match(line) {
        return Ok(Some(FrontendServerStatusMessage::Started));
    }
    Ok(None)
}

//! Module to handle running the host for the browser contributor's frontend code using `npm`.

use std::{path::PathBuf, fs::File};

use eyre::Context;
use subprocess::Redirection;


static SETUP_FRONTEND_ID: &str = "frontend";

pub struct FrontendConfiguration {
    /// Path to the setup frontend repository.
    pub frontend_repo_dir: PathBuf,
    /// Path to directory where 
    pub out_dir: PathBuf,
}

/// Run the frontend development server.
pub fn run_frontend_dev_server(
    config: FrontendConfiguration,
) -> eyre::Result<()> {
    let mut process = subprocess::Exec::cmd("npm")
        .cwd(&config.frontend_repo_dir)
        .arg("start") 
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


    // TODO: implement a thread that monitors for the server to have started.
    // I think this is the message "Files successfully emitted, waiting for typecheck results..."

    // TODO: have an integration test message bus and a MessageWaiter for the started message

    // std::thread::spawn(move || {
    //     let span = trace_span!("frontend");
    //     let _guard = span.enter();
    //
    //     let buf_pipe = BufReader::new(stdout);
    //
    //     let mut log_file = OpenOptions::new()
    //         .append(true)
    //         .create(true)
    //         .open(log_file_path)
    //         .wrap_err("unable to open log file")?;
    //
    //     // It's expected that if the process closes, the stdout will also
    //     // close and this iterator will complete gracefully.
    //     for line_result in buf_pipe.lines() {
    //         match line_result {
    //             Ok(line) => {
    //                 // Write to log file.
    //                 log_file.write_all(line.as_ref())?;
    //                 log_file.write_all("\n".as_ref())?;
    //             }
    //             Err(error) => tracing::error!(
    //                 "Error reading line from pipe to coordinator process: {}",
    //                 error
    //             ),
    //         }
    //     }
    // });

    Ok(())
}

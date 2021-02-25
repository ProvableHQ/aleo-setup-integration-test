use std::{fmt::Debug, fs::File, io::{BufRead, BufReader}, path::Path, thread::JoinHandle, time::Duration};

use flume::{Receiver, Sender};
use regex::Regex;
use subprocess::{Exec, Redirection};

use crate::{CoordinatorMessage, process::parse_exit_status};

/// A join handle for the threads created in [run_coordinator_proxy()]
pub struct SetupProxyThreadsJoin {
    listener_join: JoinHandle<()>,
    monitor_join: JoinHandle<()>,
}

impl SetupProxyThreadsJoin {
    /// Join the setup proxy server threads.
    pub fn join(self) -> std::thread::Result<()> {
        self.listener_join.join()?;
        self.monitor_join.join()
    }
}



/// Starts the nodejs proxy for the setup coordinator server.
///
/// Currently this doesn't cleanly shut down, there doesn't appear to
/// be an easy way to share the process between the line reader, and
/// the coordinator message listener.
pub fn run_coordinator_proxy<P>(
    setup_coordinator_repo: P,
    coordinator_tx: Sender<CoordinatorMessage>,
    coordinator_rx: Receiver<CoordinatorMessage>,
) -> eyre::Result<SetupProxyThreadsJoin>
where
    P: AsRef<Path> + Debug,
{
    let span = tracing::error_span!("coordinator_proxy");
    let _guard = span.enter();

    tracing::info!("Starting setup coordinator nodejs proxy.");

    let mut process = Exec::cmd("node")
        .cwd(setup_coordinator_repo)
        .arg("server.js")
        .stdout(Redirection::Pipe)
        .popen()?;

    // Extract the stdout [std::fs::File] from `process`, replacing it
    // with a None. This is needed so we can both listen to stdout and
    // interact with `process`'s mutable methods (to terminate it if
    // required).
    let mut stdout: Option<File> = None;
    std::mem::swap(&mut process.stdout, &mut stdout);
    let stdout = stdout.ok_or_else(|| eyre::eyre!("Unable to obtain nodejs process stdout"))?;

    // Thread to run the `setup_coordinator_proxy_reader()` function.
    let coordinator_tx_listener = coordinator_tx.clone();
    let listener_join = std::thread::spawn(move || {
        let span = tracing::error_span!("coordinator_proxy_listener");
        let _guard = span.enter();

        if let Err(error) = setup_coordinator_proxy_reader(stdout, coordinator_tx_listener.clone())
        {
            // tell the other threads to shut down
            let _ = coordinator_tx_listener.send(CoordinatorMessage::Shutdown);
            panic!(
                "Error while running setup coordinator nodejs proxy: {}",
                error
            );
        }

        tracing::debug!("thread closing gracefully")
    });

    // This thread monitors messages, and terminates the nodejs
    // process if a `Shutdown` message is received. It also monitors
    // the exit status of the process, and if there was an error it
    // will request a `Shutdown` and panic with the error.
    let monitor_join = std::thread::spawn(move || loop {
        let span = tracing::error_span!("coordinator_proxy_monitor");
        let _guard = span.enter();

        // Sleep occasionally because otherwise this loop will run too fast.
        std::thread::sleep(Duration::from_millis(100));

        match coordinator_rx.try_recv() {
            Ok(message) => match message {
                CoordinatorMessage::Shutdown => {
                    tracing::debug!("Telling the nodejs proxy server process to terminate");
                    process
                        .terminate()
                        .expect("error terminating nodejs proxy server process");
                }
                _ => {}
            },
            Err(flume::TryRecvError::Disconnected) => {
                panic!("coordinator_rx is disconnected");
            }
            Err(flume::TryRecvError::Empty) => {}
        }

        if let Some(exit_result) = process.poll().map(parse_exit_status) {
            tracing::debug!("nodejs proxy server process exited");
            match exit_result {
                Ok(_) => break,
                Err(error) => {
                    coordinator_tx
                        .send(CoordinatorMessage::Shutdown)
                        .expect("Error sending shutdown message");
                    panic!("Error while running nodejs proxy server: {}", error);
                }
            }
        }
    });

    Ok(SetupProxyThreadsJoin {
        listener_join,
        monitor_join,
    })
}

/// This function reads stdout from the setup coordinator nodejs proxy
/// process, and analyzes the output line by line searching for the
/// `Websocket listening is on.` message, and notifies the
/// `coordinator_rx` listeners that the proxy is ready. Also this
/// pipes the stdout from the nodejs proxy to [tracing::debug!()]
pub fn setup_coordinator_proxy_reader(
    stdout: File,
    coordinator_tx: Sender<CoordinatorMessage>,
) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

    let start_re = Regex::new("Websocket listening on.*")?;

    // It's expected that if the process closes, the stdout will also
    // close and this iterator will complete gracefully.
    for line_result in buf_pipe.lines() {
        match line_result {
            Ok(line) => {
                if start_re.is_match(&line) {
                    coordinator_tx.send(CoordinatorMessage::CoordinatorProxyReady)?;
                }

                // Pipe the process output to tracing
                tracing::debug!("{}", line);
            }
            Err(error) => {
                tracing::error!("Error reading line from pipe to nodejs process: {}", error)
            }
        }
    }

    Ok(())
}

//! Functions for starting/managing/interacting with external processes.

use std::{fs::File, thread::JoinHandle, time::Duration};

use eyre::Context;
use mpmc_bus::{Receiver, Sender, TryRecvError};
use subprocess::{Exec, Redirection};

use crate::CeremonyMessage;

/// Returns `Ok` if the `exit_status` is `Exited(0)` or `Signaled(15)`
/// (terminated by the host?), otherwise returns an `Err`.
pub fn default_parse_exit_status(exit_status: subprocess::ExitStatus) -> eyre::Result<()> {
    match exit_status {
        subprocess::ExitStatus::Exited(0) => Ok(()),
        // Terminated by the host (I'm guessing)
        subprocess::ExitStatus::Signaled(15) => Ok(()),
        unexpected => Err(eyre::eyre!(
            "Unexpected process exit status: {:?}",
            unexpected
        )),
    }
}

/// A join handle for the threads created in [wait_start_process()]
#[must_use]
pub struct MonitorProcessJoin {
    id: String,
    monitor_join: JoinHandle<()>,
    messages_join: JoinHandle<()>,
}

impl std::fmt::Debug for MonitorProcessJoin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MonitorProcessJoin({})", self.id)
    }
}

impl MonitorProcessJoin {
    /// Join the threads
    pub fn join(self) -> std::thread::Result<()> {
        let span = tracing::error_span!("join", id = %self.id);
        let _guard = span.enter();

        tracing::debug!("Joining listener thread.");
        let _ = self.monitor_join.join()?;
        tracing::debug!("Joining messages thread.");
        let _ = self.messages_join.join()?;
        tracing::debug!("Joins Completed.");

        Ok(())
    }
}

/// Join multiple [MonitorProcessJoin]s.
#[tracing::instrument(level = "error", skip(joins))]
pub fn join_multiple(mut joins: Vec<MonitorProcessJoin>) -> std::thread::Result<()> {
    while let Some(join) = joins.pop() {
        join.join()?;
        tracing::debug!("Joins remaining: {:?}", joins);
    }
    Ok(())
}

/// Starts the process specified in `exec`, with `stdout` set to
/// [Redirection::Pipe], which is fed into the specified `monitor`
/// function which runs in a new thread. Another thread is also
/// spawned which watches for [CeremonyMessage::Shutdown] and kills
/// the child process if that message is received. `parse_exit_status`
/// determines whether the returned [subprocess::ExitStatus]
/// constitutes an error, and returns an appropriate [eyre::Result].
pub fn run_monitor_process<M>(
    id: String,
    exec: Exec,
    parse_exit_status: fn(subprocess::ExitStatus) -> eyre::Result<()>,
    ceremony_tx: Sender<CeremonyMessage>,
    mut ceremony_rx: Receiver<CeremonyMessage>,
    monitor: M,
) -> eyre::Result<MonitorProcessJoin>
where
    M: Fn(File, Sender<CeremonyMessage>) + Send + Sync + 'static,
{
    tracing::info!("Starting process.");

    let mut process = exec
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

    // Thread to run the `setup_coordinator_proxy_reader()` function.
    let coordinator_tx_listener = ceremony_tx.clone();
    let monitor_span = tracing::error_span!("monitor");
    let monitor_join = std::thread::spawn(move || {
        let _guard = monitor_span.enter();

        monitor(stdout, coordinator_tx_listener.clone());

        tracing::debug!("Thread closing gracefully.")
    });

    // This thread monitors messages, and terminates the nodejs
    // process if a `Shutdown` message is received. It also monitors
    // the exit status of the process, and if there was an error it
    // will request a `Shutdown` and panic with the error.
    let messages_span = tracing::error_span!("messages");
    let messages_join = std::thread::spawn(move || {
        let _guard = messages_span.enter();

        let mut shutdown = false;

        loop {
            // Sleep occasionally because otherwise this loop will run too fast.
            std::thread::sleep(Duration::from_millis(100));

            match ceremony_rx.try_recv() {
                Ok(message) => match message {
                    CeremonyMessage::Shutdown => {
                        shutdown = true;
                    }
                    _ => {}
                },
                Err(TryRecvError::Disconnected) => {
                    panic!("`ceremony_rx` disconnected");
                }
                Err(TryRecvError::Empty) => {}
            }

            if let Some(exit_result) = process.poll().map(parse_exit_status) {
                match exit_result {
                    Ok(_) => {
                        tracing::info!("Process successfully exited.");
                        break;
                    }
                    Err(error) => {
                        ceremony_tx
                            .broadcast(CeremonyMessage::Shutdown)
                            .expect("Error sending shutdown message");
                        panic!("Error while running process: {}", error);
                    }
                }
            } else if shutdown == true {
                // This will send SIGTERM until the shutdown is
                // detected in process.poll(), just in case the
                // process has bad signal handling qualities.
                tracing::info!("Telling the process to terminate.");

                if let Err(err) = process.terminate() {
                    tracing::error!("Error while terminating process: {}. Thread closing.", err);
                    return;
                }
            }
        }

        tracing::debug!("Thread closing gracefully.")
    });

    Ok(MonitorProcessJoin {
        id,
        monitor_join,
        messages_join,
    })
}

/// Create a monitor function to be used with [run_monitor_process()] that
/// may return an [eyre::Result], if the result is an `Err` then a
/// panic will occur and the ceremony will shut down with a
/// [CeremonyMessage::Shutdown].
pub fn fallible_monitor<M>(fallible_monitor: M) -> impl Fn(File, Sender<CeremonyMessage>)
where
    M: Fn(File, Sender<CeremonyMessage>) -> eyre::Result<()> + Send + Sync + 'static,
{
    move |stdout: File, coordinator_tx: Sender<CeremonyMessage>| {
        if let Err(error) = fallible_monitor(stdout, coordinator_tx.clone()) {
            // tell the other threads to shut down
            let _ = coordinator_tx.broadcast(CeremonyMessage::Shutdown);
            // TODO: change this into something that records the fatal message, and requests a shutdown.
            // when all threads/processes have shutdown, then proceed to panic.
            panic!("Error while running process monitor: {}", error);
        }
    }
}

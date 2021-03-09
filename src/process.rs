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
    listener_join: JoinHandle<()>,
    monitor_join: JoinHandle<()>,
}

impl MonitorProcessJoin {
    /// Join the threads
    pub fn join(self) -> std::thread::Result<()> {
        let _ = self.listener_join.join()?;
        let _ = self.monitor_join.join()?;
        Ok(())
    }
}

/// Starts the process specified in `exec`, with `stdout` set to
/// [Redirection::Pipe], which is fed into the specified `monitor`
/// function which runs in a new thread. Another thread is also
/// spawned which watches for [CeremonyMessage::Shutdown] and kills
/// the child process if that message is received. `parse_exit_status`
/// determines whether the returned [subprocess::ExitStatus]
/// constitutes an error, and returns an appropriate [eyre::Result].
pub fn run_monitor_process<M>(
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
    let listener_span = tracing::error_span!("listener");
    let listener_join = std::thread::spawn(move || {
        let _guard = listener_span.enter();

        monitor(stdout, coordinator_tx_listener.clone());

        tracing::debug!("Thread closing gracefully.")
    });

    // This thread monitors messages, and terminates the nodejs
    // process if a `Shutdown` message is received. It also monitors
    // the exit status of the process, and if there was an error it
    // will request a `Shutdown` and panic with the error.
    let monitor_span = tracing::error_span!("monitor");
    let monitor_join = std::thread::spawn(move || {
        let _guard = monitor_span.enter();

        loop {
            // Sleep occasionally because otherwise this loop will run too fast.
            std::thread::sleep(Duration::from_millis(100));

            match ceremony_rx.try_recv() {
                Ok(message) => match message {
                    CeremonyMessage::Shutdown => {
                        tracing::info!("Telling the process to terminate.");
                        process
                            .terminate()
                            .expect("Error while terminating process.");
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
            }
        }

        tracing::debug!("Thread closing gracefully.")
    });

    Ok(MonitorProcessJoin {
        listener_join,
        monitor_join,
    })
}

/// Create a monitor function to be used with [monitor_process()] that
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

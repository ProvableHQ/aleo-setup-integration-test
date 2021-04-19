use std::{
    thread::JoinHandle,
    time::{Duration, Instant},
};

use humantime::format_duration;
use mpmc_bus::{Receiver, Sender, TryRecvError};

use crate::{CeremonyMessage, ShutdownReason};

/// Run a time limit thread for the specified duration. If the
/// ceremony exceeds the timer, then this will send a shutdown
/// message.
pub fn ceremony_time_limit(
    duration: std::time::Duration,
    mut ceremony_rx: Receiver<CeremonyMessage>,
    ceremony_tx: Sender<CeremonyMessage>,
) -> JoinHandle<eyre::Result<()>> {
    let duration_formatted = format_duration(duration.clone());
    let span = tracing::error_span!("time_limit", duration=%&duration_formatted);

    std::thread::spawn(move || {
        let _guard = span.enter();
        let start_time = Instant::now();

        loop {
            // Sleep occasionally because otherwise this loop will run too fast.
            std::thread::sleep(Duration::from_millis(100));

            if start_time.elapsed() > duration {
                tracing::error!("Time limit exceeded, telling ceremony to shutdown.");
                ceremony_tx.broadcast(CeremonyMessage::Shutdown(ShutdownReason::Error))?;
                return Err(eyre::eyre!(
                    "Time limit of {} for test has been exceeded.",
                    &duration_formatted
                ));
            }

            match ceremony_rx.try_recv() {
                Ok(message) => match message {
                    CeremonyMessage::Shutdown(_) => {
                        tracing::info!("Thread terminated gracefully");
                        return Ok(());
                    }
                    _ => {}
                },
                Err(TryRecvError::Disconnected) => {
                    panic!("`ceremony_rx` disconnected");
                }
                Err(TryRecvError::Empty) => {}
            }
        }
    })
}

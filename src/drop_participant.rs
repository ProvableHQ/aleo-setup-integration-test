use std::thread::JoinHandle;

use mpmc_bus::{Receiver, Sender};

use crate::CeremonyMessage;

pub fn monitor_drops(
    mut ceremony_rx: Receiver<CeremonyMessage>,
    ceremony_tx: Sender<CeremonyMessage>,
) -> JoinHandle<eyre::Result<()>> {
    let span = tracing::error_span!("monitor_drops");
    std::thread::spawn(move || {
        let _guard = span.enter();

        loop {
            match ceremony_rx.recv()? {
                CeremonyMessage::Shutdown => {
                    tracing::info!("Thread terminated gracefully");
                    return Ok(());
                }
                CeremonyMessage::ParticipantDropped(participant) => {
                    ceremony_tx.broadcast(CeremonyMessage::Shutdown)?;
                    return Err(eyre::eyre!(
                        "Participant {:?} dropped during the round.",
                        participant
                    ));
                }
                _ => {}
            }
        }
    })
}

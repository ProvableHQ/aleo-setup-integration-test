use crate::{CeremonyMessage, ContributorRef, ParticipantRef, ShutdownReason};

use mpmc_bus::{Receiver, Sender};
use serde::{Deserialize, Serialize};

use std::{collections::HashMap, thread::JoinHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropContributorConfig {
    /// Contributor is dropped (process killed) after having made this
    /// number of contributions.
    pub after_contributions: u64,
}

pub struct MonitorDropsConfig {
    pub contributor_drops: HashMap<ContributorRef, DropContributorConfig>,
}

pub fn monitor_drops(
    config: MonitorDropsConfig,
    mut ceremony_rx: Receiver<CeremonyMessage>,
    ceremony_tx: Sender<CeremonyMessage>,
) -> JoinHandle<eyre::Result<()>> {
    let mut contributor_drops = config.contributor_drops;
    let span = tracing::error_span!("monitor_drops");
    std::thread::spawn(move || {
        let _guard = span.enter();

        loop {
            match ceremony_rx.recv()? {
                CeremonyMessage::Shutdown(_) => {
                    tracing::info!("Thread terminated gracefully");
                    return Ok(());
                }
                CeremonyMessage::ParticipantDropped(participant) => {
                    match &participant {
                        ParticipantRef::Contributor(contributor) => {
                            if let Some(_drop_config) = contributor_drops.remove(&contributor) {
                                tracing::info!(
                                    "Participant {:?} dropped during the round (as expected).",
                                    &participant
                                );
                                continue;
                                // TODO: check that participant was dropped after the correct number of contributions.
                            }
                        }
                        _ => {}
                    }

                    ceremony_tx.broadcast(CeremonyMessage::Shutdown(ShutdownReason::Error))?;
                    return Err(eyre::eyre!(
                        "Participant {:?} was unexpectedly dropped during the round.",
                        &participant
                    ));
                }
                _ => {}
            }
        }
    })
}

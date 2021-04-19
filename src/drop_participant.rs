use crate::{CeremonyMessage, ContributorRef, ParticipantRef, ShutdownReason};

use mpmc_bus::{Receiver, Sender};
use serde::{Deserialize, Serialize};

use std::{collections::HashMap, thread::JoinHandle};

/// The configuration for dropping a contributor from the ceremony.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropContributorConfig {
    /// A contributor is dropped (process killed) after having made
    /// this number of contributions.
    pub after_contributions: u64,
}

/// Configuration for running [monitor_drops()].
pub struct MonitorDropsConfig {
    /// Expected dropped contributors. If not all drops specified here
    /// have occurred by the time the [monitor_drops()] thread shuts
    /// down at the end of the test, then an error will be returned
    /// during join.
    pub contributor_drops: HashMap<ContributorRef, DropContributorConfig>,
}

/// Monitor the ceremony for dropped contributors. Returns an error if
/// an unexpected drop occurs or if not all expected drops have
/// occurred.
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
                CeremonyMessage::Shutdown(reason) => {
                    if let ShutdownReason::TestFinished = reason {
                        if !contributor_drops.is_empty() {
                            return Err(eyre::eyre!(
                                "The specified drops did not occur as \
                                    expected during the ceremony: {:?}",
                                contributor_drops
                            ));
                        }
                    }

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

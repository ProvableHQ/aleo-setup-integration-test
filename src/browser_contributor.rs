//! Testing for a contributor using the `setup-frontend` running in a web browser.

use std::{path::PathBuf, time::Duration};

use mpmc_bus::{Receiver, Sender};
use thirtyfour::{By, DesiredCapabilities, WebDriver};
use tracing::{trace_span, Instrument};
use url::Url;

use crate::{
    drop_participant::DropContributorConfig, test::ContributorStartConfig, CeremonyMessage,
};

#[derive(Clone)]
pub struct BrowserContributor {
    /// A short id used to reference this contributor with the
    /// integration test. See [Contributor::coordinator_id()] for the id
    /// that the coordinator uses to refer to the contributor.
    pub id: String,
}

// impl BrowserContributor {
//     /// The id used to reference this contributor by the coordinator,
//     /// and within the ceremony transcript.
//     pub fn id_on_coordinator(&self) -> String {
//         format!("{}.contributor", self.address)
//     }
//
//     /// Obtains the [ContributorRef] referring to this [Contributor].
//     pub fn as_contributor_ref(&self) -> ContributorRef {
//         ContributorRef {
//             address: self.address.clone(),
//         }
//     }
// }

/// Configuration for running a contributor.
#[derive(Debug, Clone)]
pub struct BrowserContributorConfig {
    /// An identifier for this contributor, used only by the
    /// integration test, also used as the name of the working
    /// directory for this contributor.
    pub id: String,
    /// The url to the frontend being hosted by [crate::frontend].
    pub frontend_url: Url,
    /// The out directory for the ceremony, the working directory for
    /// this contributor is `out_dir`/`id`.
    pub out_dir: PathBuf,
    /// The drop configuration for this contributor. If `Some`, then
    /// the contributor will be dropped (via killing the process)
    /// according to the specified config. If `None` then the
    /// contributor will not be deliberately dropped from the round,
    /// and if it is dropped, an error will occur.
    pub drop: Option<DropContributorConfig>,
    /// When this contributor is configured to start during the round.
    pub start: ContributorStartConfig,
}

// TODO: so we need to start a server to host the frontend. Then we need to start each contributor
// in its own browser instance most likely.

pub fn run_browser_contributor(
    config: BrowserContributorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    mut ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<()> {
    let span = trace_span!("browser_contributor", id = config.id.as_str());
    std::thread::spawn::<_, eyre::Result<()>>(move || {
        let _guard = span.enter();
        let rt = tokio::runtime::Builder::new_multi_thread().build()?;

        let spawned_ceremony_rx = ceremony_rx.clone();
        let spawn_span = trace_span!("tokio");
        rt.spawn(
            async move { spawn_browser_contributor(config, ceremony_tx, spawned_ceremony_rx) }
                .instrument(spawn_span),
        );

        loop {
            match ceremony_rx.recv()? {
                CeremonyMessage::Shutdown(_) => {
                    rt.shutdown_timeout(Duration::from_secs(10));
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    });
    Ok(())

    // TODO: to get the aleo address I think I might need to extract that from the UI and return it
    // from here.
}

async fn spawn_browser_contributor(
    config: BrowserContributorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    mut ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<()> {
    let capabilities = DesiredCapabilities::firefox();

    tracing::info!("Starting WebDriver.");
    let driver = WebDriver::new("http://localhost:4444", capabilities).await?;

    tracing::info!("Connecting to frontend.");
    driver.get(config.frontend_url).await?;
    let element = driver
        .find_element(By::XPath("/html/body/div/section/div[1]/div[1]/div[1]/h1"))
        .await?;
    tracing::info!("Found: {}", element.text().await?);

    Ok(())
}

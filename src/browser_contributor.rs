//! Testing for a contributor using the `setup-frontend` running in a web browser.

use std::{path::PathBuf, time::Duration};

use mpmc_bus::{Receiver, Sender};
use thirtyfour::{By, DesiredCapabilities, WebDriver};
use tracing::Instrument;
use url::Url;

use crate::{specification, CeremonyMessage};

#[derive(Debug, Clone)]
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
    pub drop: Option<specification::DropContributor>,
    /// When this contributor is configured to start during the round.
    pub start: specification::ContributorStart,
}

// TODO: so we need to start a server to host the frontend. Then we need to start each contributor
// in its own browser instance most likely.

pub fn run_browser_contributor(
    config: BrowserContributorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    mut ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<()> {
    let span = tracing::error_span!("browser_contributor", id = config.id.as_str());
    std::thread::spawn::<_, eyre::Result<()>>(move || {
        let thread_span = span.clone();
        let _guard = thread_span.enter();
        let rt = tokio::runtime::Builder::new_multi_thread().build()?;

        let spawned_ceremony_rx = ceremony_rx.clone();
        rt.spawn(
            async move { spawn_browser_contributor(config, ceremony_tx, spawned_ceremony_rx) }
                .instrument(span),
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

// TODO: these are the steps that need to be performed
//
// TODO: first steps
//
//
// AFTER COMPLETED CEREMONY
//
// Check that path /html/body/div/section/div[1]/div/section/main/div/h3
// Is equal to "Success! Thank you for your contribution to the setup."
//
// Click button /html/body/div/section/div[1]/div/section/main/div/div/button[1]/span
// Check that the span contains "Back Up Wallet"
//
// Check that title /html/body/div[1]/section/div[1]/div/section/main/div/div[1]/h3
// Is equal to "Back Up using Private Key"
//
// Check that /html/body/div[1]/section/div[1]/div/section/main/div/div[1]/div[1]/p[1]
// Contains a valid Aleo Private Key: e.g.
// APrivateKey1uaaXYKve7R9BvVZtYXcXAbmFPfX2vVP2m7nhyUpXq5STAi3
//
// Check that /html/body/div[1]/section/div[1]/div/section/main/div/div[1]/div[1]/p[2]
// Contains a valid Aleo Address: e.g.
// aleo1lypf27q7dsq0rdjx7de4v6g9rjj22xzl9kdz5dt8w8sukxvhgyrqfwf8ea
//
// Click /html/body/div[1]/section/div[1]/div/section/main/div/div[1]/div[2]/button[1]/span
// Check that the span contains "Copy"
// Check the system clipboard?
//
// Click /html/body/div[1]/section/div[1]/div/section/main/div/div[1]/div[2]/button[2]/span
// Check that the span contains "Download"
// Check that recieved json file with the name backup.json:
// {"address":"aleo1lypf27q7dsq0rdjx7de4v6g9rjj22xzl9kdz5dt8w8sukxvhgyrqfwf8ea","privateKey":"APrivateKey1uaaXYKve7R9BvVZtYXcXAbmFPfX2vVP2m7nhyUpXq5STAi3"}
//
//
// Click /html/body/div[1]/section/div[1]/div/section/main/div/div[2]/div[2]/button/span
// Check that the span contains "Continue"
//
// Click /html/body/div[4]/div/div[2]/div/div[2]/div/div/div[2]/button[2]/span
// Check that the span contains "I Understand"
//
// Check the title /html/body/div[1]/section/div[1]/div/section/main/div/div[1]/h3
// Contains: "You've successfully backed up your wallet."
//
// Click /html/body/div[1]/section/div[1]/div/section/main/div/div[2]/div[2]/button/span
// Check that the span contains "Continue"
//
// Check that the title
// Contains: "Thank you for your contribution to the setup."
//
// TODO: Check what happens when there is no email entered (validation)
//
// Enter an email address in the input field at xpath //*[@id="email"]
//
// Click /html/body/div[1]/section/div[1]/div/section/main/div/div[2]/div[2]/button/span
// Check that the span contains "Continue"
//
// TODO: mock the email submission

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

//! Testing for a contributor using the `setup-frontend` running in a web browser.

use std::{convert::TryInto, path::PathBuf, pin::Pin, time::Duration};

use eyre::Context;
use mpmc_bus::{Receiver, Sender};
use std::future::Future;
use thirtyfour::{error::WebDriverError, By, DesiredCapabilities, WebDriver, WebElement};
use tracing::{Instrument, Span};
use url::Url;

use crate::{join::MultiJoinable, specification, CeremonyMessage, ShutdownReason};

#[derive(Debug, Clone)]
pub struct BrowserContributor {
    /// A short id used to reference this contributor with the
    /// integration test. See [Contributor::coordinator_id()] for the id
    /// that the coordinator uses to refer to the contributor.
    pub id: String,
}

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
    /// See [specification::BrowserTestMode].
    pub mode: specification::BrowserTestMode,
}

/// Used to wait for browser contributor to complete (or error out).
#[derive(Debug)]
#[must_use]
pub struct BrowserContributorJoin {
    run_join: std::thread::JoinHandle<()>,
}

impl BrowserContributorJoin {
    /// Joins the threads created by [run_contributor()].
    fn join(self) -> std::thread::Result<()> {
        self.run_join.join()
    }
}

impl MultiJoinable for BrowserContributorJoin {
    fn join(self: Box<Self>) -> std::thread::Result<()> {
        BrowserContributorJoin::join(*self)
    }
}

// TODO: so we need to start a server to host the frontend. Then we need to start each contributor
// in its own browser instance most likely.

pub fn run_browser_contributor(
    config: BrowserContributorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<Option<BrowserContributorJoin>> {
    let span = tracing::error_span!("browser_contributor", id = config.id.as_str());
    let thread_span = span.clone();
    let _guard = span.enter();
    tracing::info!("Starting browser contributor: {}", &config.id);

    if let specification::BrowserTestMode::Manual(manual_settings) = config.mode {
        if let Some(launch) = manual_settings.launch {
            tracing::info!(
                "Launching browser contributor to be run manually on URL: {}",
                &config.frontend_url
            );
            webbrowser::open_browser(launch.try_into()?, config.frontend_url.as_ref())
                .wrap_err("Unable to open Manual browser contributor URL in the browser")?;
        } else {
            tracing::info!(
                "Please open the URL to run the browser contributor manually: {}",
                &config.frontend_url
            );
        }
        return Ok(None);
    }

    tracing::debug!("Browser contributor will be controlled automatically.");
    let run_join = std::thread::spawn(move || {
        let thread_span_2 = thread_span.clone();
        let _guard = thread_span.enter();

        let result = run_async(thread_span_2, ceremony_tx.clone(), ceremony_rx, config);

        if let Err(_) = &result {
            // There is already an error broadcast in the run client task, this is just in case
            // starting the async executor failed.
            ceremony_tx
                .broadcast(CeremonyMessage::Shutdown(ShutdownReason::Error))
                .expect("error broadcasting");
        }

        result.expect("Error running browser contributor");
    });

    // TODO: to get the aleo address I think I might need to extract that from the UI and return it
    // from here.

    Ok(Some(BrowserContributorJoin { run_join }))
}

fn run_async(
    span: Span,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    config: BrowserContributorConfig,
) -> eyre::Result<()> {
    let async_span = span.clone();
    tracing::debug!("Starting a new tokio runtime.");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .wrap_err("Error starting tokio runtime")?;

    rt.block_on(
        async move { run_tasks(async_span, ceremony_tx, ceremony_rx, config).await }
            .instrument(span),
    )
}

async fn run_tasks(
    span: Span,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
    config: BrowserContributorConfig,
) -> eyre::Result<()> {
    let client_span = span.clone();
    let run_ceremony_rx = ceremony_rx.clone();
    let run_client_join = tokio::task::spawn(
        async move {
            let result = run_webdriver_client(config, ceremony_tx.clone(), run_ceremony_rx).await;
            if let Err(_) = &result {
                // Tell the ceremony to shutdown, to allow all the joins to be resolved and the
                // error to resolve as a process panic.
                ceremony_tx
                    .broadcast(CeremonyMessage::Shutdown(ShutdownReason::Error))
                    .expect("error broadcasting");
            }

            result
        }
        .instrument(client_span),
    );

    let monitor_span = span.clone();
    let monitor_join = tokio::task::spawn_blocking::<_, eyre::Result<()>>(move || {
        let mut ceremony_rx = ceremony_rx;
        let _guard = monitor_span.enter();
        let monitor_span2 = tracing::error_span!("monitor");
        let _guard2 = monitor_span2.enter();

        loop {
            match ceremony_rx
                .recv()
                .wrap_err("Error receiving ceremony message")?
            {
                CeremonyMessage::Shutdown(_) => {
                    // break should cause the select to short circuit and cancel running the
                    // webdriver client and server.
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    });

    tokio::select! {
        result = run_client_join => result?,
        result = monitor_join => result?,
    }
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

/// Wait for the client to report that the server is running.
async fn wait_setup_running(driver: &WebDriver) -> eyre::Result<()> {
    tracing::debug!("Waiting for the client to report if the setup is running.");
    loop {
        // There is an assumption here that the ceremony will not complete in less than 5 seconds!
        tokio::time::sleep(Duration::from_secs(5)).await;
        tracing::debug!("Attempting to find running title...");
        let title_element: WebElement = match driver.find_element(By::XPath("//main/div/h3")).await
        {
            Ok(ok) => ok,
            Err(WebDriverError::NoSuchElement(_)) => continue,
            Err(unexpected_error) => return Err(unexpected_error.into()),
        };

        let title_text = title_element.text().await?;
        if title_text.contains("setup is running") {
            tracing::debug!("Found running title {:?}", title_text);
            return Ok(());
        }
    }
}

/// Run the client via [`WebDriver`].
async fn run_webdriver_client(
    config: BrowserContributorConfig,
    ceremony_tx: Sender<CeremonyMessage>,
    ceremony_rx: Receiver<CeremonyMessage>,
) -> eyre::Result<()> {
    let span = tracing::debug_span!("client");
    let _guard = span.enter();
    tracing::debug!("Starting WebDriver.");
    let capabilities = DesiredCapabilities::firefox();

    let driver = WebDriver::new("http://localhost:4444", capabilities).await?;

    tracing::debug!("Performing HTTP GET to start the frontend client.");
    driver.get(config.frontend_url).await?;
    tracing::debug!("Started the frontend client.");

    wait_setup_running(&driver).await?;
    tracing::info!("Client reports that the setup is running.");

    // TODO
    Err(eyre::eyre!(
        "This test has not been completely implemented yet"
    ))
}

//! Testing for a contributor using the `setup-frontend` running in a web browser.

use std::path::PathBuf;

use crate::{drop_participant::DropContributorConfig, test::ContributorStartConfig};

/// Configuration for running a contributor.
#[derive(Debug, Clone)]
pub struct BrowserContributor {
    /// An identifier for this contributor, used only by the
    /// integration test, also used as the name of the working
    /// directory for this contributor.
    pub id: String,
    /// The path to the binary to run this contributor.
    pub contributor_bin_path: PathBuf,
    /// The url to connect to the coordinator.
    pub coordinator_api_url: String,
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

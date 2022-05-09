//! Format of the specification of the tests to be run. See
//! [example-specification.ron](../example-specification.ron) for an annotated example of the
//! specification file that the code in this module supports deserializing.

use std::convert::TryInto;

use serde::{Deserialize, Serialize};

use crate::Environment;

/// Specification for multiple tests to be performed. Will be
/// deserialized from a ron file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Specification {
    /// Specifications for the individual tests.
    pub tests: Vec<SingleTest>,
}

pub type TestId = String;

/// Options for each individual test in the [Specification]'s `tests`
/// field.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct SingleTest {
    /// Id for the individual test.
    pub id: TestId,

    /// Number of verifier participants for the test.
    pub verifiers: u8,

    /// (Optional) Number of replacement contributors for the test.
    /// Default: 0
    #[serde(default = "default_replacement_contributors")]
    pub replacement_contributors: u8,

    /// What environment to use for the setup.
    pub environment: Environment,

    /// (Optional) Time limit for this individual test (in seconds).
    /// Exceeding this will cause the test to fail. If set to
    /// `None`  then there is no time limit. Default: `None`
    #[serde(default)]
    pub timout: Option<u64>,

    /// (Optional) Whether to skip running this test. Default:
    /// `false`.
    #[serde(default = "skip_default")]
    pub skip: bool,

    /// Configure the tests performed for each round of the ceremony.
    pub rounds: Vec<TestRound>,
}

/// Start a ceremony participant after
/// [StartAfterContributions::contributions] have been made in the
/// current round.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartAfterRoundContributions {
    /// See [StartAfterContributions].
    pub after_round_contributions: u64,
}

/// The configuration for when a contributor will be started
/// during/before a round.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ContributorStart {
    /// Start the contributor at the beginning of the ceremony. This
    /// is only a valid option for replacement contributors.
    CeremonyStart,
    /// Start the contributor while the current round is waiting for
    /// participants to join.
    RoundStart,
    // See [StartAfterContributions].
    AfterRoundContributions(StartAfterRoundContributions),
}

impl Default for ContributorStart {
    fn default() -> Self {
        Self::RoundStart
    }
}

/// What type of contributor will be started.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ContributorKind {
    /// Browser Contributor.
    Browser(BrowserSettings),
    /// CLI Contributor.
    CLI,
}

/// Settings for launching a browser contributor.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BrowserSettings {
    /// See [`BrowserTestMode`].
    #[serde(default)]
    pub test_mode: BrowserTestMode,
}

/// What type of web browser to use for manual browser.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum LaunchBrowser {
    ///Operating system's default browser
    Default,

    ///Mozilla Firefox
    Firefox,

    ///Google Chrome
    Chrome,

    ///Mac OS Safari
    Safari,
}

impl TryInto<webbrowser::Browser> for LaunchBrowser {
    type Error = eyre::Error;
    fn try_into(self) -> Result<webbrowser::Browser, eyre::Error> {
        Ok(match self {
            LaunchBrowser::Default => webbrowser::Browser::Default,
            LaunchBrowser::Firefox => webbrowser::Browser::Firefox,
            LaunchBrowser::Chrome => webbrowser::Browser::Chrome,
            LaunchBrowser::Safari => webbrowser::Browser::Safari,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualBrowserSettings {
    /// If `Some`, then a browser will be automatically launched to run the contributor. Which
    /// browser is launched depends on the chosen [`LaunchBrowser`].
    pub launch: Option<LaunchBrowser>,
}

impl Default for LaunchBrowser {
    fn default() -> Self {
        Self::Default
    }
}

/// What mode to run the browser contributor in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BrowserTestMode {
    Automatic,
    Manual(ManualBrowserSettings),
}

impl Default for BrowserTestMode {
    fn default() -> Self {
        Self::Automatic
    }
}

/// The configuration for dropping a contributor from the ceremony.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropContributor {
    /// A contributor is dropped (process killed) after having made
    /// this number of contributions.
    pub after_contributions: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributor {
    /// See [`ContributorType`].
    pub kind: ContributorKind,
    /// See [`ContributorStartConfig`].
    #[serde(default)]
    pub start: ContributorStart,
    /// See [`DropContributorConfig`].
    #[serde(default)]
    pub drop: Option<DropContributor>,
}

/// Specification for running each round of the ceremony.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestRound {
    /// Specification for each contributor that will be started for this round.
    pub contributors: Vec<Contributor>,
}

/// Default value for [SingleTestOptions::replacement_contributors].
fn default_replacement_contributors() -> u8 {
    0
}

/// Default value for [SingleTestOptions::skip].
fn skip_default() -> bool {
    false
}

#[cfg(test)]
mod test {
    use super::Specification;

    /// Test deserializing `example-config.ron` to [Specification].
    #[test]
    fn test_deserialize_example() {
        let example_string = std::fs::read_to_string("example-specification.ron")
            .expect("Error while reading example-specification.ron file");
        let _example: Specification =
            ron::from_str(&example_string).expect("Error while deserializing example-config.ron");
    }
}

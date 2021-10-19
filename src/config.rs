//! This module contains functions for running multiple integration
//! tests.

use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use serde::Deserialize;

use crate::{
    test::{
        default_aleo_setup, default_aleo_setup_coordinator, default_aleo_setup_state_monitor, Repo,
        TestRound,
    },
    Environment,
};

/// Configuration for how to run the tests.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Remove any artifacts created during a previous integration
    /// test run before starting.
    pub clean: bool,

    /// Whether or not to build the components being tested. By
    /// default this is `true`.  Setting this to `false` makes the
    /// test faster for development purposes.
    #[serde(default = "default_build")]
    pub build: bool,

    /// Keep the git repositories. The following effects take place
    /// when this is enabled:
    ///
    /// + Don't delete git repositories if [Options::clean] is
    ///   enabled.
    pub keep_repos: bool,

    /// Whether to install install prerequisites. By default this is
    /// `true`. Setting this to `false` makes the test faster for
    /// development purposes.
    #[serde(default = "default_install_prerequisites")]
    pub install_prerequisites: bool,

    /// Whether to run the `aleo-setup-state-monitor` application.
    /// Requires `python3` and `pip` to be installed. Only supported
    /// on Linux.
    pub state_monitor: bool,

    /// Path to where the log files, key files and transcripts are stored.
    pub out_dir: PathBuf,

    /// The code repository for the `aleo-setup` project.
    ///
    /// Example [Repo::Remote] specification:
    ///
    /// ```ron
    /// aleo_setup_state_monitor_repo: (
    ///     type: "Remote",
    ///     dir: "aleo-setup-state-monitor",
    ///     url: "git@github.com:AleoHQ/aleo-setup-state-monitor.git",
    ///     branch: "include-build",
    /// ),
    /// ```
    ///
    /// Example [Repo::Local] specification:
    ///
    /// ```ron
    /// aleo_setup_repo: (
    ///     type: "Local",
    ///     dir: "../aleo-setup",
    /// ),
    /// ```
    #[serde(default = "default_aleo_setup")]
    pub aleo_setup_repo: Repo,

    /// The code repository for the `aleo-setup-coordinator` project.
    ///
    /// See [SingleTestOptions::aleo_setup_repo] for useage examples.
    #[serde(default = "default_aleo_setup_coordinator")]
    pub aleo_setup_coordinator_repo: Repo,

    /// The code repository for the `aleo-setup-state-monitor` project.
    ///
    /// See [SingleTestOptions::aleo_setup_repo] for useage examples.
    #[serde(default = "default_aleo_setup_state_monitor")]
    pub aleo_setup_state_monitor_repo: Repo,

    /// The address used for the `aleo-setup-state-monitor` web
    /// server. By default `127.0.0.1:5001`.
    #[serde(default = "default_state_monitor_address")]
    pub state_monitor_address: SocketAddr,
}

fn default_build() -> bool {
    true
}

fn default_install_prerequisites() -> bool {
    true
}

fn default_state_monitor_address() -> SocketAddr {
    SocketAddr::from_str("127.0.0.1:5001").unwrap()
}

pub type TestId = String;

/// Options for each individual test in the [Specification]'s `tests`
/// field.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SingleTestOptions {
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

/// Default value for [TestOptions::replacement_contributors].
fn default_replacement_contributors() -> u8 {
    0
}

fn skip_default() -> bool {
    false
}

#[cfg(test)]
mod test {
    use super::Config;

    /// Test deserializing `example-config.ron` to [Specification].
    #[test]
    fn test_deserialize_example() {
        let example_string = std::fs::read_to_string("example-config.ron")
            .expect("Error while reading example-config.ron file");
        let _example: Config =
            ron::from_str(&example_string).expect("Error while deserializing example-config.ron");
    }
}

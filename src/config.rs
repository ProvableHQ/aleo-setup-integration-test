//! This module contains functions for running multiple integration
//! tests.

use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use serde::Deserialize;

use crate::{
    git::RemoteGitRepo,
    test::{Repo, StateMonitorOptions, TestRound},
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
    #[serde(default = "default_state_monitor")]
    pub state_monitor: Option<StateMonitorConfig>,

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
    #[serde(default = "default_aleo_setup_repo")]
    pub aleo_setup_repo: Repo,

    /// The code repository for the `aleo-setup-coordinator` project.
    ///
    /// See [SingleTestOptions::aleo_setup_repo] for useage examples.
    #[serde(default = "default_aleo_setup_coordinator_repo")]
    pub aleo_setup_coordinator_repo: Repo,
}

#[derive(Deserialize, Debug, Clone)]
pub struct StateMonitorConfig {
    /// The code repository for the `aleo-setup-state-monitor` project.
    ///
    /// See [SingleTestOptions::aleo_setup_repo] for useage examples.
    #[serde(default = "default_aleo_setup_state_monitor_repo")]
    pub repo: Repo,
    /// The address used for the `aleo-setup-state-monitor` web
    /// server. By default `127.0.0.1:5001`.
    #[serde(default = "default_state_monitor_address")]
    pub address: SocketAddr,
}

impl Into<StateMonitorOptions> for StateMonitorConfig {
    fn into(self) -> StateMonitorOptions {
        StateMonitorOptions {
            repo: self.repo,
            address: self.address,
        }
    }
}

impl Default for StateMonitorConfig {
    fn default() -> Self {
        Self {
            repo: default_aleo_setup_state_monitor_repo(),
            address: default_state_monitor_address(),
        }
    }
}

/// Default value for [Config::aleo_setup_repo].
pub fn default_aleo_setup_repo() -> Repo {
    Repo::Remote(RemoteGitRepo {
        dir: "aleo-setup".into(),
        url: "git@github.com:AleoHQ/aleo-setup.git".into(),
        branch: "master".into(),
    })
}

/// Default value for [Config::aleo_setup_coordinator_repo].
pub fn default_aleo_setup_coordinator_repo() -> Repo {
    Repo::Remote(RemoteGitRepo {
        dir: "aleo-setup-coordinator".into(),
        url: "https://github.com/AleoHQ/aleo-setup.git".into(),
        branch: "main".into(),
    })
}

/// Default value for [Config::state_monitor].
pub fn default_state_monitor() -> Option<StateMonitorConfig> {
    Some(Default::default())
}

/// Default value for [Config::aleo_setup_state_monitor_repo].
pub fn default_aleo_setup_state_monitor_repo() -> Repo {
    Repo::Remote(RemoteGitRepo {
        dir: "aleo-setup-state-monitor".into(),
        url: "git@github.com:AleoHQ/aleo-setup-state-monitor.git".into(),
        branch: "include-build".into(), // branch to include build files so that npm is not required
    })
}

/// Default value for [Config::build].
fn default_build() -> bool {
    true
}

/// Default value for [Config::install_prerequisites].
fn default_install_prerequisites() -> bool {
    true
}

/// Default value for [Config::state_monitor_address].
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

    /// Test deserializing `default-config.ron` to [Specification].
    #[test]
    fn test_deserialize_default() {
        let example_string = std::fs::read_to_string("default-config.ron")
            .expect("Error while reading default-config.ron file");
        let _example: Config =
            ron::from_str(&example_string).expect("Error while deserializing example-config.ron");
    }
}

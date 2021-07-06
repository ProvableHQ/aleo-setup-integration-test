//! This module contains functions for running multiple integration
//! tests.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use color_eyre::Section;
use eyre::Context;
use serde::Deserialize;

use crate::{
    reporting::LogFileWriter,
    test::{
        default_aleo_setup, default_aleo_setup_coordinator, default_aleo_setup_state_monitor,
        integration_test, Repo, TestOptions, TestRound,
    },
    util::create_dir_if_not_exists,
    Environment,
};

/// Specification for multiple tests to be performed. Will be
/// deserialized from a ron file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Specification {
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

    /// Specifications for the individual tests.
    pub tests: Vec<SingleTestOptions>,
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

/// Options for each individual test in the [Specification]'s `tests`
/// field.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SingleTestOptions {
    /// Id for the individual test.
    pub id: String,

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

/// Run multiple tests specified in the ron specification file.
pub fn run_test_specification(
    specification_file: impl AsRef<Path>,
    log_writer: &LogFileWriter,
) -> eyre::Result<()> {
    tracing::info!(
        "Running multiple integration tests using specification {:?}",
        specification_file.as_ref()
    );

    let specification_string = std::fs::read_to_string(specification_file.as_ref())
        .wrap_err_with(|| eyre::eyre!("Error while reading specification ron file"))?;

    let specification: Specification = ron::from_str(&specification_string)
        .wrap_err_with(|| eyre::eyre!("Error while parsing specification ron file"))?;

    if specification.tests.len() == 0 {
        return Err(eyre::eyre!(
            "Expected at least one test to be defined in the specification file."
        ));
    }

    let out_dir = specification.out_dir.clone();

    // Perfom the clean action if required.
    if specification.clean {
        tracing::info!("Cleaning integration test.");

        if out_dir.exists() {
            tracing::info!("Removing out dir: {:?}", out_dir);
            std::fs::remove_dir_all(&out_dir)?;
        }
    }

    create_dir_if_not_exists(&out_dir)?;

    let mut errors: Vec<eyre::Error> = specification
        .tests
        .iter()
        .filter(|options| {
            if options.skip {
                tracing::info!("Skipping test {}", options.id);
                false
            } else {
                true
            }
        })
        .enumerate()
        .map(|(i, options)| {
            let test_id = &options.id;
            let out_dir = out_dir.join(test_id);

            dbg!(&options);

            // The first test uses the keep_repos and no_prereqs
            // option. Subsequent tests do not clean, and do not
            // attempt to install prerequisites.
            let test_options = if i == 0 {
                TestOptions {
                    clean: false,
                    build: specification.build,
                    keep_repos: specification.keep_repos,
                    install_prerequisites: specification.install_prerequisites,
                    replacement_contributors: options.replacement_contributors,
                    verifiers: options.verifiers,
                    out_dir,
                    environment: options.environment,
                    state_monitor: specification.state_monitor,
                    timout: options.timout.map(Duration::from_secs),
                    aleo_setup_repo: specification.aleo_setup_repo.clone(),
                    aleo_setup_coordinator_repo: specification.aleo_setup_coordinator_repo.clone(),
                    aleo_setup_state_monitor_repo: specification
                        .aleo_setup_state_monitor_repo
                        .clone(),
                    rounds: options.rounds.clone(),
                    state_monitor_address: specification.state_monitor_address.clone(),
                }
            } else {
                TestOptions {
                    clean: false,
                    build: false,
                    keep_repos: true,
                    install_prerequisites: true,
                    replacement_contributors: options.replacement_contributors,
                    verifiers: options.verifiers,
                    out_dir,
                    environment: options.environment,
                    state_monitor: specification.state_monitor,
                    timout: options.timout.map(Duration::from_secs),
                    aleo_setup_repo: specification.aleo_setup_repo.clone(),
                    aleo_setup_coordinator_repo: specification.aleo_setup_coordinator_repo.clone(),
                    aleo_setup_state_monitor_repo: specification
                        .aleo_setup_state_monitor_repo
                        .clone(),
                    rounds: options.rounds.clone(),
                    state_monitor_address: specification.state_monitor_address.clone(),
                }
            };

            (test_id, test_options)
        })
        .map(|(id, options)| {
            let span = tracing::error_span!("test", id=%id);
            let _guard = span.enter();

            tracing::info!("Running integration test with id {:?}", id);

            integration_test(&options, log_writer)
                .map(|test_results| {
                    let test_results_str =
                        ron::ser::to_string_pretty(&test_results, Default::default())
                            .expect("Unable to serialize test results");
                    tracing::info!("Test results: \n {}", test_results_str);
                })
                .wrap_err_with(|| {
                    eyre::eyre!("Error while running individual test with id: {:?}", id)
                })
        })
        .filter(Result::is_err)
        .map(Result::unwrap_err)
        .map(|error| {
            // Display error message for each error that occurs during individual tests.
            tracing::error!("{:?}", error);
            error
        })
        .collect();

    let n_errors = errors.len();

    // Grab the last error which will be the one actually returned by this method.
    let last_error = errors.pop();

    let result = match last_error {
        Some(error) => Err(error),
        None => Ok(()),
    };

    match n_errors {
        1 => {
            result.wrap_err_with(|| eyre::eyre!("Error during one of the integration tests"))
        }
        _ => {
            result.wrap_err_with(|| eyre::eyre!("Errors during {} of the integration tests", n_errors))
                .with_note(||
                    format!("{} errors have occurred. This error shows the trace for the last error that occurred. \
                    Check the stdout log for ERROR trace messages for other errors.", n_errors))
        }
    }
}

#[cfg(test)]
mod test {
    use super::Specification;

    /// Test deserializing `example-config.ron` to [Specification].
    #[test]
    fn test_deserialize_example() {
        let example_string = std::fs::read_to_string("example-config.ron")
            .expect("Error while reading example-config.ron file");
        let _example: Specification =
            ron::from_str(&example_string).expect("Error while deserializing example-config.ron");
    }
}

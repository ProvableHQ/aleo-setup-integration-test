//! This module contains functions for running multiple integration
//! tests.

use std::time::Duration;

use color_eyre::Section;
use eyre::Context;
use serde::Deserialize;

use crate::{
    config::Config,
    reporting::LogFileWriter,
    test::{integration_test, TestOptions, TestRound},
    util::create_dir_if_not_exists,
    Environment,
};

/// Specification for multiple tests to be performed. Will be
/// deserialized from a ron file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Specification {
    /// Specifications for the individual tests.
    pub tests: Vec<SingleTestOptions>,
}

pub type TestId = String;

/// Options for each individual test in the [Specification]'s `tests`
/// field.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct SingleTestOptions {
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

impl Specification {
    /// Run multiple tests specified in the ron specification file.
    ///
    /// If `only_tests` contains some values, only the test id's contained
    /// within this vector will be run. This will override the test's skip
    /// value.
    pub fn run(
        &self,
        config: &Config,
        only_tests: &[TestId],
        log_writer: &LogFileWriter,
    ) -> eyre::Result<()> {
        if self.tests.len() == 0 {
            return Err(eyre::eyre!(
                "Expected at least one test to be defined in the specification file."
            ));
        }

        let out_dir = config.out_dir.clone();

        // Perfom the clean action if required.
        if config.clean {
            tracing::info!("Cleaning integration test.");

            if out_dir.exists() {
                tracing::info!("Removing out dir: {:?}", out_dir);
                std::fs::remove_dir_all(&out_dir)?;
            }
        }

        create_dir_if_not_exists(&out_dir)?;

        let mut errors: Vec<eyre::Error> = self
            .tests
            .iter()
            .filter(|options| {
                if !only_tests.is_empty() {
                    only_tests.contains(&options.id)
                } else {
                    if options.skip {
                        tracing::info!("Skipping test {}", options.id);
                        false
                    } else {
                        true
                    }
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
                        build: config.build,
                        keep_repos: config.keep_repos,
                        install_prerequisites: config.install_prerequisites,
                        replacement_contributors: options.replacement_contributors,
                        verifiers: options.verifiers,
                        out_dir,
                        environment: options.environment,
                        state_monitor: config.state_monitor.clone().map(Into::into),
                        timout: options.timout.map(Duration::from_secs),
                        aleo_setup_repo: config.aleo_setup_repo.clone(),
                        aleo_setup_coordinator_repo: config.aleo_setup_coordinator_repo.clone(),
                        rounds: options.rounds.clone(),
                    }
                } else {
                    TestOptions {
                        clean: false,
                        build: false,
                        keep_repos: true,
                        install_prerequisites: false,
                        replacement_contributors: options.replacement_contributors,
                        verifiers: options.verifiers,
                        out_dir,
                        environment: options.environment,
                        state_monitor: config.state_monitor.clone().map(Into::into),
                        timout: options.timout.map(Duration::from_secs),
                        aleo_setup_repo: config.aleo_setup_repo.clone(),
                        aleo_setup_coordinator_repo: config.aleo_setup_coordinator_repo.clone(),
                        rounds: options.rounds.clone(),
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

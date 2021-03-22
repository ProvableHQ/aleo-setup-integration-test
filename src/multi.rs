//! This module contains functions for running multiple integration
//! tests.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use color_eyre::Section;
use eyre::Context;
use serde::Deserialize;

use crate::{
    test::{run_integration_test, TestOptions},
    util::create_dir_if_not_exists,
    Environment,
};

/// Specification for multiple tests to be performed. Will be
/// deserialized from a json file.
#[derive(Deserialize, Debug)]
struct Specification {
    /// Remove any artifacts created during a previous integration
    /// test run before starting.
    pub clean: bool,

    /// Keep the git repositories. The following effects take place
    /// when this is enabled:
    ///
    /// + Don't delete git repositories if [Options::clean] is
    ///   enabled.
    pub keep_repos: bool,

    /// If true, don't attempt to install install prerequisites. Makes
    /// the test faster for development purposes.
    pub no_prereqs: bool,

    /// Whether to run the `aleo-setup-state-monitor` application.
    /// Requires `python3` and `pip` to be installed. Only supported
    /// on Linux.
    pub state_monitor: bool,

    /// Path to where the log files, key files and transcripts are stored.
    pub out_dir: PathBuf,

    /// Specifications for the individual tests.
    pub tests: Vec<SingleTestOptions>,
}

/// Options for each individual test in the [Specification]'s `tests`
/// field.
#[derive(Deserialize, Debug)]
struct SingleTestOptions {
    /// Id for the individual test.
    pub id: String,

    /// Number of contributor participants for the test.
    pub contributors: u8,

    /// Number of verifier participants for the test.
    pub verifiers: u8,

    /// What environment to use for the setup.
    pub environment: Environment,

    /// Timout (in seconds) for running a ceremony round of the
    /// integration test (not including setting up prerequisites). If
    /// this time is exceeded for a given round, the test will fail.
    #[serde(default)]
    pub round_timout: Option<u64>,
}

/// Run multiple tests specified in the json specification file.
pub fn run_multi_test(specification_file: impl AsRef<Path>) -> eyre::Result<()> {
    let specification_string = std::fs::read_to_string(specification_file.as_ref())
        .wrap_err_with(|| eyre::eyre!("Error while reading specification json file"))?;

    let specification: Specification = serde_json::from_str(&specification_string)
        .wrap_err_with(|| eyre::eyre!("Error while parsing specification json file"))?;

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
        .enumerate()
        .map(|(i, options)| {
            tracing::info!("test {}", i);
            let test_id = &options.id;
            let out_dir = out_dir.join(test_id);

            let test_options = if i == 0 {
                TestOptions {
                    clean: false,
                    keep_repos: specification.keep_repos,
                    no_prereqs: specification.no_prereqs,
                    contributors: options.contributors,
                    verifiers: options.verifiers,
                    out_dir,
                    environment: options.environment,
                    state_monitor: specification.state_monitor,
                    round_timout: options.round_timout.map(Duration::from_secs),
                }
            } else {
                TestOptions {
                    clean: false,
                    keep_repos: true,
                    no_prereqs: true,
                    contributors: options.contributors,
                    verifiers: options.verifiers,
                    out_dir,
                    environment: options.environment,
                    state_monitor: specification.state_monitor,
                    round_timout: options.round_timout.map(Duration::from_secs),
                }
            };

            (test_id, test_options)
        })
        .map(|(id, options)| {
            let span = tracing::error_span!("test", id=%id);
            let _guard = span.enter();

            tracing::info!("Running integration test with id {:?}", id);

            run_integration_test(&options)
                .map(|test_results| {
                    let test_results_str = serde_json::to_string_pretty(&test_results)
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

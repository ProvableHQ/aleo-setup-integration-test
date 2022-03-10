use color_eyre::Help;
use eyre::Context;
use std::time::Duration;
use url::Url;

use crate::{
    config::{Config, TestId},
    frontend::{start_frontend_dev_server, FrontendConfiguration},
    git::clone_git_repository,
    npm::{check_node_version, npm_install},
    reporting::LogFileWriter,
    rust::{build_rust_crate, install_rust_toolchain, RustToolchain},
    specification::Specification,
    test::{integration_test, Repo, TestOptions},
    util::create_dir_if_not_exists,
};

/// The currently supported node version.
static SUPPORTED_NODE_MAJOR_VERSION: u8 = 16;

/// Clean integration test directory, and remove git repositories if required by
/// [Config::keep_repos] option.
pub fn clean(config: &Config) -> eyre::Result<()> {
    tracing::info!("Cleaning integration test.");
    let out_dir = &config.out_dir;
    if out_dir.exists() {
        tracing::info!("Removing out dir: {:?}", out_dir);
        fs_err::remove_dir_all(&out_dir)?;
    }

    if !config.keep_repos {
        if let Repo::Remote(repo) = &config.aleo_setup_repo {
            if repo.dir.exists() {
                tracing::info!("Removing `aleo-setup` repository: {:?}.", &repo.dir);
                fs_err::remove_dir_all(&repo.dir)?;
            }
        }
        if let Repo::Remote(repo) = &config.aleo_setup_coordinator_repo {
            if repo.dir.exists() {
                tracing::info!(
                    "Removing `aleo-setup-coordinator` repository: {:?}.",
                    &repo.dir
                );
                fs_err::remove_dir_all(&repo.dir)?;
            }
        }

        if let Repo::Remote(repo) = &config.setup_frontend_repo {
            if repo.dir.exists() {
                tracing::info!("Removing `setup-frontend` repository: {:?}.", &repo.dir);
                fs_err::remove_dir_all(&repo.dir)?;
            }
        }
    }

    Ok(())
}

/// Configuration for running [build()], building the projects required to run the ceremony.
pub struct BuildConfig<'a> {
    run_config: &'a Config,
    frontend_required: bool,
}

/// Build the projects required to run the ceremony.
pub fn build(config: &BuildConfig) -> eyre::Result<()> {
    let BuildConfig {
        run_config,
        frontend_required,
    } = *config;
    let rust_toolchain = RustToolchain::Stable;
    if run_config.install_prerequisites {
        tracing::info!("Installing toolchain prerequisites.");
        // Install a specific version of the rust toolchain needed to be
        // able to compile `aleo-setup`.
        install_rust_toolchain(&rust_toolchain).wrap_err_with(|| {
            eyre::eyre!("error while installing rust toolchain {}", rust_toolchain)
        })?;
    }

    tracing::info!("Building required projects.");

    let coordinator_dir = run_config.aleo_setup_coordinator_repo.dir();
    let setup_dir = run_config.aleo_setup_repo.dir();
    let setup_frontend_dir = run_config.setup_frontend_repo.dir();
    // Build the setup coordinator Rust project.
    build_rust_crate(coordinator_dir, &rust_toolchain)
        .wrap_err("error while building aleo-setup-coordinator crate")?;

    // Build the setup1-contributor Rust project.
    build_rust_crate(setup_dir.join("setup1-contributor"), &rust_toolchain)
        .wrap_err("error while building setup1-contributor crate")?;

    // Build the setup1-verifier Rust project.
    build_rust_crate(setup_dir.join("setup1-verifier"), &rust_toolchain)
        .wrap_err("error while building setup1-verifier crate")?;

    // Build the setup1-cli-tools Rust project.
    build_rust_crate(setup_dir.join("setup1-cli-tools"), &rust_toolchain)
        .wrap_err("error while building setup1-verifier crate")?;

    let node_version = check_node_version()?;
    if node_version.major != SUPPORTED_NODE_MAJOR_VERSION {
        return Err(eyre::eyre!(
            "Unsupported node version {}, expected v{}.*",
            node_version,
            SUPPORTED_NODE_MAJOR_VERSION
        ));
    }

    if frontend_required {
        if run_config.install_prerequisites {
            npm_install(setup_frontend_dir).wrap_err("error while building setup-frontend")?;
        }
        let frontend_env_path = setup_frontend_dir.join(".env");
        if !frontend_env_path.exists() {
            fs_err::write(&frontend_env_path, "SKIP_PREFLIGHT_CHECK=true").wrap_err_with(|| {
                format!(
                    "Error while writing to .env file {:?} for setup-frontend",
                    &frontend_env_path
                )
            })?;
        }
    }

    if let Some(state_monitor_options) = &run_config.state_monitor {
        // Build the aleo-setup-state-monitor Rust project.
        build_rust_crate(state_monitor_options.repo.dir(), &RustToolchain::Stable)
            .wrap_err("error while building aleo-setup-state-monitor server crate")?;
    }

    Ok(())
}

/// Clone the git repos for the projects required to run the ceremony.
pub fn clone_git_repos(config: &Config) -> eyre::Result<()> {
    tracing::info!("Cloning git repositories (if required).");
    if let Repo::Remote(repo) = &config.aleo_setup_coordinator_repo {
        tracing::info!("Cloning aleo-setup-coordinator git repository.");
        clone_git_repository(repo)
            .wrap_err("Error while cloning `aleo-setup-coordinator` git repository.")?;
    }

    if let Repo::Remote(repo) = &config.aleo_setup_repo {
        tracing::info!("Cloning aleo-setup git repository.");
        clone_git_repository(repo).wrap_err("Error while cloning `aleo-setup` git repository.")?;
    }

    if let Repo::Remote(repo) = &config.setup_frontend_repo {
        tracing::info!("Cloning setup-frontend git repository.");
        clone_git_repository(repo)
            .wrap_err("Error while cloning `setup-frontend` git repository.")?;
    }

    if let Some(state_monitor_options) = config.state_monitor.as_ref() {
        if let Repo::Remote(remote_repo) = &state_monitor_options.repo {
            tracing::info!("Cloning aleo-setup-state-monitor git repository.");
            clone_git_repository(remote_repo)
                .wrap_err("Error while cloning `aleo-setup-state-monitor` git repository.")?;
        }
    }

    Ok(())
}

/// Returns `true` if the frontend is required for any of the tests.
fn frontend_required(specification: &Specification) -> bool {
    specification
        .tests
        .iter()
        .flat_map(|test| test.rounds.iter())
        .find(|round| round.browser_contributors > 0)
        .is_some()
}

/// Run multiple tests specified in the ron specification file.
///
/// If `only_tests` contains some values, only the test id's contained
/// within this vector will be run. This will override the test's skip
/// value.
pub fn run(
    specification: &Specification,
    config: &Config,
    only_tests: &[TestId],
    log_writer: &LogFileWriter,
) -> eyre::Result<()> {
    if specification.tests.is_empty() {
        return Err(eyre::eyre!(
            "Expected at least one test to be defined in the specification file."
        ));
    }

    log_writer.set_no_out_file();

    // Perfom the clean action if required.
    if config.clean {
        clean(config)?;
    }

    let out_dir = config.out_dir.clone();
    create_dir_if_not_exists(&out_dir)?;

    // Create the log file, and write out the options that were used to run this test.
    log_writer.set_out_file(out_dir.join("integration-test.log"))?;

    let frontend_required = frontend_required(specification);

    // Attempt to clone the git repos if they don't already exist.
    clone_git_repos(config)?;

    if config.build {
        let build_config = BuildConfig {
            run_config: &config,
            frontend_required,
        };
        build(&build_config)?;
    }

    let frontend_out_dir = config.out_dir.join("frontend");

    if frontend_required {
        create_dir_if_not_exists(&frontend_out_dir)?;
        let frontend_config = FrontendConfiguration {
            frontend_repo_dir: config.setup_frontend_repo.dir().to_path_buf(),
            out_dir: frontend_out_dir,
            backend_url: Url::parse("http://localhost:9000")?,
        };
        // TODO: use the returned control and status channels to shutdown the server gracefully at
        // the end of this test.
        start_frontend_dev_server(frontend_config)?;
    }

    let mut errors: Vec<eyre::Error> = specification
        .tests
        .iter()
        .filter(|options| {
            if !only_tests.is_empty() {
                only_tests.contains(&options.id)
            } else if options.skip {
                tracing::info!("Skipping test {}", options.id);
                false
            } else {
                true
            }
        })
        .map(|options| {
            let test_id = &options.id;
            let out_dir = out_dir.join(test_id);

            // The first test uses the keep_repos and no_prereqs
            // option. Subsequent tests do not clean, and do not
            // attempt to install prerequisites.
            let test_options = TestOptions {
                replacement_contributors: options.replacement_contributors,
                verifiers: options.verifiers,
                out_dir,
                environment: options.environment,
                state_monitor: config.state_monitor.clone().map(Into::into),
                timout: options.timout.map(Duration::from_secs),
                aleo_setup_repo: config.aleo_setup_repo.clone(),
                aleo_setup_coordinator_repo: config.aleo_setup_coordinator_repo.clone(),
                setup_frontend_repo: config.setup_frontend_repo.clone(),
                rounds: options.rounds.clone(),
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

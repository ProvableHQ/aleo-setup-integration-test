//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use aleo_setup_integration_test::{
    config::Config,
    options::CmdOptions,
    reporting::{setup_reporting, LogFileWriter},
    specification::{Specification, TestId},
};

use eyre::Context;
use structopt::StructOpt;

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
fn main() -> eyre::Result<()> {
    let log_writer = LogFileWriter::new();
    let _guard = setup_reporting(log_writer.clone())?;

    let options: CmdOptions = CmdOptions::from_args();

    let only_tests: Vec<TestId> = match &options.id {
        Some(id) => vec![id.clone()],
        None => Vec::new(),
    };

    let config_string = match &options.config_file {
        Some(config_file) => {
            tracing::info!("Loading configuration from file: {:?}", &config_file);
            std::fs::read_to_string(&config_file)
                .wrap_err_with(|| eyre::eyre!("Error while reading specification ron file"))?
        }
        None => {
            tracing::info!("Using default configuration.");
            include_str!("../default-config.ron").to_owned()
        }
    };

    let config: Config = ron::from_str(&config_string)
        .wrap_err_with(|| eyre::eyre!("Error while parsing configuration"))?;

    tracing::info!(
        "Running integration test using specification {:?}",
        &options.specification_file
    );

    let specification_string = std::fs::read_to_string(&options.specification_file)
        .wrap_err_with(|| eyre::eyre!("Error while reading specification ron file"))?;

    let specification: Specification =
        ron::from_str(&specification_string).wrap_err_with(|| {
            eyre::eyre!(
                "Error while parsing test specification {:?}",
                &options.specification_file
            )
        })?;

    let result = specification
        .run(&config, &only_tests, &log_writer)
        .wrap_err_with(|| {
            eyre::eyre!(
                "Error while running tests specified in {:?}",
                &options.specification_file
            )
        });

    // report the error to tracing and log file
    if let Err(error) = &result {
        tracing::error!("{}", error);
    }

    result
}

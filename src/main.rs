//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use aleo_setup_integration_test::{
    options::CmdOptions,
    reporting::{setup_reporting, LogFileWriter},
    specification::{run_test_specification, TestId},
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

    let result = run_test_specification(&only_tests, &options.specification_file, &log_writer)
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

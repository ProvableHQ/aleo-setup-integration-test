//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use std::convert::TryFrom;

use aleo_setup_integration_test::{
    multi::run_multi_test,
    options::{CmdOptions, Command},
    reporting::{setup_reporting, LogFileWriter},
    test::{run_integration_test, TestOptions},
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

    let result = match options.cmd {
        Some(Command::Multi(multi_options)) => {
            run_multi_test(&multi_options.specification_file, &log_writer).wrap_err_with(|| {
                eyre::eyre!(
                    "Error while running tests specified in {:?}",
                    &multi_options.specification_file
                )
            })
        }
        None => run_integration_test(&TestOptions::try_from(&options)?, &log_writer).map(|_| ()),
    };

    // report the error to tracing and log file
    if let Err(error) = &result {
        tracing::error!("{}", error);
    }

    result
}

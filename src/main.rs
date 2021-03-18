//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use std::convert::TryFrom;

use aleo_setup_integration_test::{
    options::CmdOptions,
    reporting::setup_reporting,
    test::{run_integration_test, TestOptions},
};

use structopt::StructOpt;

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
fn main() -> eyre::Result<()> {
    setup_reporting()?;

    let options: CmdOptions = CmdOptions::from_args();

    run_integration_test(&TestOptions::try_from(&options)?)?;

    Ok(())
}

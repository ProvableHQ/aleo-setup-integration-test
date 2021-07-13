//! Options for running the integration test.

use std::path::PathBuf;

use serde::Serialize;
use structopt::StructOpt;

use crate::specification::TestId;

/// Command line options for running the Aleo Setup integration test.
/// More complex options (such as drops) are available via the `multi`
/// command interface by specifying the test in `ron` format.
#[derive(Debug, StructOpt, Serialize)]
#[structopt(
    name = "Aleo Setup Integration Test",
    about = "An integration test for the aleo-setup and aleo-setup-coordinator repositories."
)]
pub struct CmdOptions {
    /// ron file specifying the test options.
    pub specification_file: PathBuf,
    /// Test with only a specific test id contained within the
    /// specification file.
    #[structopt(long = "id")]
    pub id: Option<TestId>,
}

//! Options for running the integration test.

use std::path::PathBuf;

use serde::Serialize;
use structopt::StructOpt;

use crate::Environment;
/// Command line options for running the Aleo Setup integration test.
#[derive(Debug, StructOpt, Serialize)]
#[structopt(
    name = "Aleo Setup Integration Test",
    about = "An integration test for the aleo-setup and aleo-setup-coordinator repositories."
)]
pub struct CmdOptions {
    /// Remove any artifacts created during a previous integration
    /// test run before starting.
    #[structopt(long, short = "c")]
    pub clean: bool,

    /// Keep the git repositories. The following effects take place
    /// when this is enabled:
    ///
    /// + Don't delete git repositories if [Options::clean] is
    ///   enabled.
    #[structopt(long, short = "k")]
    pub keep_repos: bool,

    /// Don't attempt to install install prerequisites. Makes the test
    /// faster for development purposes.
    #[structopt(long, short = "n")]
    pub no_prereqs: bool,

    /// Number of contributor participants for the test.
    #[structopt(long, default_value = "1")]
    pub contributors: u8,

    /// Number of verifier participants for the test.
    #[structopt(long, default_value = "1")]
    pub verifiers: u8,

    /// Path to where the log files, key files and transcripts are stored.
    #[structopt(long, short = "o", default_value = "out")]
    pub out_dir: PathBuf,

    /// What environment to use for the setup.
    #[structopt(
        long,
        default_value = Environment::str_variants()[0],
        possible_values = Environment::str_variants(),
    )]
    pub environment: Environment,

    /// Whether to run the `aleo-setup-state-monitor` application.
    /// Requires `python3` and `pip` to be installed. Only supported
    /// on Linux.
    #[structopt(long)]
    pub state_monitor: bool,
}

//! Functions for interacting with `npm` the nodejs package manager.

use crate::process::default_parse_exit_status;

use subprocess::Exec;

use std::path::Path;

/// Run `npm install` in the specified `run_directory`.
#[tracing::instrument(level = "error")]
pub fn npm_install(run_directory: impl AsRef<Path> + std::fmt::Debug) -> eyre::Result<()> {
    Exec::cmd("npm")
        .cwd(run_directory)
        .arg("install")
        .join()
        .map_err(eyre::Error::from)
        .map_err(|error| error.wrap_err(format!("Error running `npm install`")))
        .and_then(default_parse_exit_status)
}

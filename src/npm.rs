use crate::process::default_parse_exit_status;

use subprocess::Exec;

use std::path::Path;

/// Run `npm install` in the specified `run_directory`.
pub fn npm_install<P>(run_directory: P) -> eyre::Result<()>
where
    P: AsRef<Path>,
{
    Exec::cmd("npm")
        .cwd(run_directory)
        .arg("install")
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)
}

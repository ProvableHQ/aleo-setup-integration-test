//! Functions for interacting with `npm` the nodejs package manager.

use crate::process::default_parse_exit_status;

use regex::{Captures, Regex};
use subprocess::Exec;

use std::{fmt::Display, path::Path};

/// Run `npm install` in the specified `run_directory`.
#[tracing::instrument(level = "error")]
pub fn npm_install(run_directory: impl AsRef<Path> + std::fmt::Debug) -> eyre::Result<()> {
    Exec::cmd("npm")
        .cwd(run_directory)
        .arg("install")
        .join()
        .map_err(eyre::Error::from)
        .map_err(|error| error.wrap_err("Error running `npm install`"))
        .and_then(default_parse_exit_status)
}

pub struct NodeVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl Display for NodeVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "v{}.{}.{}",
            self.major, self.minor, self.patch
        ))
    }
}

lazy_static::lazy_static! {
    static ref NODE_VERSION_RE: Regex = Regex::new(r"v(\d+)\.(\d+)\.(\d+)").unwrap();
}

/// Check the nodejs version.
pub fn check_node_version() -> eyre::Result<NodeVersion> {
    let version_string = Exec::cmd("node")
        .arg("--version")
        .capture()?
        .stdout_str()
        .strip_suffix('\n')
        .ok_or_else(|| eyre::eyre!("Unable to strip prefix"))?
        .to_owned();
    let groups: Captures = NODE_VERSION_RE
        .captures(&version_string)
        .ok_or_else(|| eyre::eyre!("Unable to parse node version with regex"))?;

    let mut versions: [u8; 3] = [0; 3];

    for (i, item) in versions.iter_mut().enumerate() {
        *item = groups
            .get(i + 1)
            .expect("capture group 1 is not present in regex")
            .as_str()
            .parse()?;
    }

    Ok(NodeVersion {
        major: versions[0],
        minor: versions[1],
        patch: versions[2],
    })
}

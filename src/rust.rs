//! Functions for interacting with Rust projects, and managing the
//! Rust toolchain.

use std::{fmt::Debug, path::Path};

use subprocess::Exec;

use crate::process::default_parse_exit_status;

/// A rust toolchain version/specification to use with `cargo` or
/// `rustup` command line tools.
#[derive(Debug, Clone)]
pub enum RustToolchain {
    /// The `rustup` system default Rust toolchain version.
    SystemDefault,
    /// The currently installed stable Rust toolchain version.
    Stable,
    /// The currently installed beta Rust toolchain version.
    Beta,
    /// The currently installed nightly Rust toolchain version.
    Nightly,
    /// A specific Rust toolchain version. e.g. `nightly-2020-08-15`
    /// or `1.48`.
    Specific(String),
}

impl std::fmt::Display for RustToolchain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustToolchain::SystemDefault => write!(f, "default"),
            RustToolchain::Stable => write!(f, "stable"),
            RustToolchain::Beta => write!(f, "beta"),
            RustToolchain::Nightly => write!(f, "nightly"),
            RustToolchain::Specific(toolchain) => write!(f, "{}", toolchain),
        }
    }
}

impl Default for RustToolchain {
    fn default() -> Self {
        Self::SystemDefault
    }
}

/// Install a version of the rust toolchain using `rustup`.
pub fn install_rust_toolchain(toolchain: &RustToolchain) -> eyre::Result<()> {
    let cmd = Exec::cmd("rustup").arg("toolchain").arg("install");

    let cmd = match toolchain {
        RustToolchain::SystemDefault => Err(eyre::eyre!(
            "Invalid argument for `toolchain`: SystemDefault"
        )),
        _ => Ok(cmd.arg(toolchain.to_string())),
    }?;

    default_parse_exit_status(cmd.join()?)
}

/// Build a rust crate at the specified `crate_dir` using `cargo` with
/// the specified Rust `toolchain` version.
///
/// The returned path is the output directory, containing the build
/// artifacts.
#[tracing::instrument(level = "error")]
pub fn build_rust_crate<P>(crate_dir: P, toolchain: &RustToolchain) -> eyre::Result<()>
where
    P: AsRef<Path> + Debug,
{
    tracing::info!("Building crate");

    let cmd = Exec::cmd("cargo").cwd(&crate_dir);

    let cmd = match toolchain {
        RustToolchain::SystemDefault => cmd,
        _ => cmd.arg(format!("+{}", toolchain)),
    };

    cmd.arg("build")
        .arg("--release")
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)?;

    Ok(())
}

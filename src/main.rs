use subprocess::Exec;
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

use std::{fmt::Debug, path::{Path, PathBuf}};

/// Returns `Ok` if the `exit_status` is 0, otherwise returns an `Err`.
fn parse_exit_status(exit_status: subprocess::ExitStatus) -> eyre::Result<()> {
    match exit_status {
        subprocess::ExitStatus::Exited(0) => Ok(()),
        unexpected => Err(eyre::eyre!(
            "Unexpected process exit status: {:?}",
            unexpected
        )),
    }
}

/// Obtain clone/download a git repository.
///
/// + `repository_url` is the path to the github repository: e.g
///   `git@github.com:ExampleUser/example_repo.git`.
/// + `target_dir` is the directory where the repository will be
///   placed. e.g. `target_dir`.
#[tracing::instrument(level = "error")]
fn get_git_repository<P>(repository_url: &str, target_dir: P) -> eyre::Result<()>
where
    P: AsRef<Path> + Debug,
{
    tracing::info!("Cloning repository");
    let exit_status = Exec::cmd("git")
        .arg("clone")
        .arg(repository_url)
        .args(&["--depth", "1"])
        .arg(target_dir.as_ref())
        .join()?;

    parse_exit_status(exit_status)
}

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

/// Build a rust crate at the specified `crate_dir` using `cargo` with
/// the specified Rust `toolchain` version.
///
/// The returned path is the output directory, containing the build
/// artifacts.
#[tracing::instrument(level = "error")]
fn build_rust_crate<P>(crate_dir: P, toolchain: &RustToolchain) -> eyre::Result<PathBuf>
where
    P: AsRef<Path> + Debug {
    tracing::info!("Building crate");

    let cmd = Exec::cmd("cargo")
        .cwd(&crate_dir);

    let cmd = match toolchain {
        RustToolchain::SystemDefault => cmd,
        _ => {
            cmd.arg(format!("+{}", toolchain))
        }
    };

    let exit_status = cmd
        .arg("build")
        .arg("--release")
        .join()?;
    
    parse_exit_status(exit_status)?;

    Ok(crate_dir.as_ref().join("target/release"))
}

/// Set up [tracing] and [color-eyre](color_eyre).
fn setup_reporting() -> eyre::Result<()> {
    color_eyre::install()?;

    let fmt_layer = tracing_subscriber::fmt::layer();
    let error_layer = tracing_error::ErrorLayer::default();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(error_layer)
        .init();

    Ok(())
}

/// Install a version of the rust toolchain using `rustup`.
fn install_rust_toolchain(toolchain: &RustToolchain) -> eyre::Result<()> {
    let cmd = Exec::cmd("rustup")
        .arg("toolchain")
        .arg("install");

    let cmd = match toolchain {
        RustToolchain::SystemDefault => {
            Err(eyre::eyre!("Invalid argument for `toolchain`: SystemDefault"))
        }
        _ => {
            Ok(cmd.arg(toolchain.to_string()))
        }
    }?;

    let exit_status = cmd.join()?;

    parse_exit_status(exit_status)
}

const ALEO_SETUP_COORDINATOR_DIR: &str = "aleo-setup-coordinator";
const ALEO_SETUP_DIR: &str = "aleo-setup";

fn main() -> eyre::Result<()> {
    setup_reporting()?;

    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());
    install_rust_toolchain(&rust_1_47_nightly)?;

    get_git_repository(
        "https://github.com/AleoHQ/aleo-setup-coordinator",
        ALEO_SETUP_COORDINATOR_DIR,
    )?;
    get_git_repository("https://github.com/AleoHQ/aleo-setup", ALEO_SETUP_DIR)?;
    
    build_rust_crate(ALEO_SETUP_COORDINATOR_DIR, &rust_1_47_nightly)?;

    Ok(())
}

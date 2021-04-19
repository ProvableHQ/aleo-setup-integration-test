//! Functions for interacting with git repositories.

use crate::process::default_parse_exit_status;
use std::path::Path;
use subprocess::Exec;

/// Performas a shallow (`--depth 1`) clone of a git repository.
///
/// + `repository_url` is the path to the github repository: e.g
///   `git@github.com:ExampleUser/example_repo.git`.
/// + `target_dir` is the directory where the repository will be
///   placed. e.g. `target_dir`.
/// + `branch` is the branch to checkout.
#[tracing::instrument(level = "error")]
pub fn clone_git_repository<P>(
    repository_url: &str,
    target_dir: P,
    branch: &str,
) -> eyre::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    if target_dir.as_ref().exists() {
        tracing::info!(
            "Git repository already cloned to {:?}, skipping.",
            target_dir
        );
        return Ok(());
    }

    tracing::info!("Cloning git repository.");

    Exec::cmd("git")
        .arg("clone")
        .arg(repository_url)
        .args(&["--depth", "1"])
        .args(&["--branch", branch])
        .arg(target_dir.as_ref())
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)
}

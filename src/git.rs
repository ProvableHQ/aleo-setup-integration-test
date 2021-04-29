//! Functions for interacting with git repositories.

use crate::process::default_parse_exit_status;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use subprocess::Exec;

/// A git repository which will be cloned from a remote url.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteGitRepo {
    /// Which directory the git repo will be cloned to.
    pub dir: PathBuf,
    /// What url to use for the git repository.
    pub url: String,
    /// What branch to use for the git repository
    pub branch: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocalGitRepo {
    /// Which directory the git repo is currently located in.
    pub dir: PathBuf,
}

/// Performas a shallow (`--depth 1`) clone of a git repository.
///
/// + `repository_url` is the path to the github repository: e.g
///   `git@github.com:ExampleUser/example_repo.git`.
/// + `target_dir` is the directory where the repository will be
///   placed. e.g. `target_dir`.
/// + `branch` is the branch to checkout.
#[tracing::instrument(level = "error")]
pub fn clone_git_repository(repo: &RemoteGitRepo) -> eyre::Result<()> {
    if repo.dir.exists() {
        tracing::info!("Git repository already cloned to {:?}, skipping.", repo.dir);
        return Ok(());
    }

    tracing::info!("Cloning git repository.");

    Exec::cmd("git")
        .arg("clone")
        .arg(&repo.url)
        .args(&["--depth", "1"])
        .args(&["--branch", &repo.branch])
        .arg(&repo.dir)
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)
}

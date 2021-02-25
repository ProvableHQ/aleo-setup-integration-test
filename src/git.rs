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

    Exec::cmd("git")
        .arg("clone")
        .arg(repository_url)
        .args(&["--depth", "1"])
        .arg(target_dir.as_ref())
        .join()
        .map_err(eyre::Error::from)
        .and_then(parse_exit_status)
}

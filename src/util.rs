use std::path::{Path, PathBuf};

use eyre::Context;

/// Create a directory if it doesn't yet exist, and return it as a
/// [PathBuf].
pub fn create_dir_if_not_exists<P>(path: P) -> eyre::Result<PathBuf>
where
    P: AsRef<Path> + Into<PathBuf>,
{
    if !path.as_ref().exists() {
        fs_err::create_dir(&path)
            .wrap_err_with(|| format!("Error while creating path {:?}.", path.as_ref()))?;
    }
    Ok(path.into())
}

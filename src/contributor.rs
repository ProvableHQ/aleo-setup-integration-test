use std::{ffi::OsStr};

use crate::process::default_parse_exit_status;

/// Generate the contributor key file.
pub fn generate_contributor_key<PB: AsRef<OsStr>, PK: AsRef<OsStr>>(contributor_bin_path: PB, key_file: PK) -> eyre::Result<()> {
    subprocess::Exec::cmd(contributor_bin_path)
        .arg("generate")
        .arg(key_file)
        .join()
        .map_err(eyre::Error::from)
        .and_then(default_parse_exit_status)
}
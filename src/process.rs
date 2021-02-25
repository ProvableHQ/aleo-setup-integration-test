/// Returns `Ok` if the `exit_status` is 0, otherwise returns an `Err`.
pub fn parse_exit_status(exit_status: subprocess::ExitStatus) -> eyre::Result<()> {
    match exit_status {
        subprocess::ExitStatus::Exited(0) => Ok(()),
        // Terminated by the host (I'm guessing)
        subprocess::ExitStatus::Signaled(15) => Ok(()),
        unexpected => Err(eyre::eyre!(
            "Unexpected process exit status: {:?}",
            unexpected
        )),
    }
}
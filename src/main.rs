//! Integration test for `aleo-setup-coordinator` and `aleo-setup`'s
//! `setup1-contributor` and `setup1-verifier`.

use flume::{Receiver, Sender};
use regex::Regex;
use subprocess::{Exec, Redirection};
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

use std::{
    fmt::{Debug, Display},
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    str::FromStr,
    thread::JoinHandle,
    time::Duration,
};

/// Returns `Ok` if the `exit_status` is 0, otherwise returns an `Err`.
fn parse_exit_status(exit_status: subprocess::ExitStatus) -> eyre::Result<()> {
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
        .and_then(parse_exit_status)?;

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
    let cmd = Exec::cmd("rustup").arg("toolchain").arg("install");

    let cmd = match toolchain {
        RustToolchain::SystemDefault => Err(eyre::eyre!(
            "Invalid argument for `toolchain`: SystemDefault"
        )),
        _ => Ok(cmd.arg(toolchain.to_string())),
    }?;

    parse_exit_status(cmd.join()?)
}

/// Run `npm install` in the specified `run_directory`.
fn npm_install<P>(run_directory: P) -> eyre::Result<()>
where
    P: AsRef<Path>,
{
    Exec::cmd("npm")
        .cwd(run_directory)
        .arg("install")
        .join()
        .map_err(eyre::Error::from)
        .and_then(parse_exit_status)
}

/// Message sent between the various components running during the
/// setup ceremony. Each component will have a process monitor running
/// in its own thread which will listen to these messages.
#[derive(Clone, Debug, Copy)]
enum CoordinatorMessage {
    /// Notify the receivers that the coordinator rocket server is
    /// ready to start receiving requests.
    CoordinatorReady,
    /// Notify the receivers that the cordinator nodejs proxy is ready
    /// to start receiving requests.
    CoordinatorProxyReady,
    /// Tell all the recievers to shut down.
    Shutdown,
}

/// This function reads stdout from the setup coordinator nodejs proxy
/// process, and analyzes the output line by line searching for the
/// `Websocket listening is on.` message, and notifies the
/// `coordinator_rx` listeners that the proxy is ready.
fn setup_coordinator_proxy_reader(
    stdout: File,
    coordinator_tx: Sender<CoordinatorMessage>,
) -> eyre::Result<()> {
    let buf_pipe = BufReader::new(stdout);

    let start_re = Regex::new("Websocket listening on.*")?;

    // It's expected that if the process closes, the stdout will also
    // close and this iterator will complete gracefully.
    for line_result in buf_pipe.lines() {
        match line_result {
            Ok(line) => {
                if start_re.is_match(&line) {
                    coordinator_tx.send(CoordinatorMessage::CoordinatorProxyReady)?;
                }

                tracing::debug!("{}", line);
            }
            Err(error) => {
                tracing::error!("Error reading line from pipe to nodejs process: {}", error)
            }
        }
    }

    Ok(())
}

/// A join handle for the threads created in [run_coordinator_proxy()]
struct SetupProxyThreadsJoin {
    listener_join: JoinHandle<()>,
    monitor_join: JoinHandle<()>,
}

impl SetupProxyThreadsJoin {
    /// Join the setup proxy server threads.
    pub fn join(self) -> std::thread::Result<()> {
        self.listener_join.join()?;
        self.monitor_join.join()
    }
}

/// Starts the nodejs proxy for the setup coordinator server.
///
/// Currently this doesn't cleanly shut down, there doesn't appear to
/// be an easy way to share the process between the line reader, and
/// the coordinator message listener.
fn run_coordinator_proxy<P>(
    setup_coordinator_repo: P,
    coordinator_tx: Sender<CoordinatorMessage>,
    coordinator_rx: Receiver<CoordinatorMessage>,
) -> eyre::Result<SetupProxyThreadsJoin>
where
    P: AsRef<Path> + Debug,
{
    let span = tracing::error_span!("coordinator_proxy");
    let _guard = span.enter();

    tracing::info!("Starting setup coordinator nodejs proxy.");

    let mut process = Exec::cmd("node")
        .cwd(setup_coordinator_repo)
        .arg("server.js")
        .stdout(Redirection::Pipe)
        .popen()?;

    // Extract the stdout [std::fs::File] from `process`, replacing it
    // with a None. This is needed so we can both listen to stdout and
    // interact with `process`'s mutable methods (to terminate it if
    // required).
    let mut stdout: Option<File> = None;
    std::mem::swap(&mut process.stdout, &mut stdout);
    let stdout = stdout.ok_or_else(|| eyre::eyre!("Unable to obtain nodejs process stdout"))?;

    // Thread to run the `setup_coordinator_proxy_reader()` function.
    let coordinator_tx_listener = coordinator_tx.clone();
    let listener_join = std::thread::spawn(move || {
        let span = tracing::error_span!("coordinator_proxy_listener");
        let _guard = span.enter();

        if let Err(error) = setup_coordinator_proxy_reader(stdout, coordinator_tx_listener.clone())
        {
            // tell the other threads to shut down
            let _ = coordinator_tx_listener.send(CoordinatorMessage::Shutdown);
            panic!(
                "Error while running setup coordinator nodejs proxy: {}",
                error
            );
        }

        tracing::debug!("thread closing gracefully")
    });

    // This thread monitors messages, and terminates the nodejs
    // process if a `Shutdown` message is received. It also monitors
    // the exit status of the process, and if there was an error it
    // will request a `Shutdown` and panic with the error.
    let monitor_join = std::thread::spawn(move || loop {
        let span = tracing::error_span!("coordinator_proxy_monitor");
        let _guard = span.enter();

        // Sleep occasionally because otherwise this loop will run too fast.
        std::thread::sleep(Duration::from_millis(100));

        match coordinator_rx.try_recv() {
            Ok(message) => match message {
                CoordinatorMessage::Shutdown => {
                    tracing::debug!("Telling the nodejs proxy server process to terminate");
                    process
                        .terminate()
                        .expect("error terminating nodejs proxy server process");
                }
                _ => {}
            },
            Err(flume::TryRecvError::Disconnected) => {
                panic!("coordinator_rx is disconnected");
            }
            Err(flume::TryRecvError::Empty) => {}
        }

        if let Some(exit_result) = process.poll().map(parse_exit_status) {
            tracing::debug!("nodejs proxy server process exited");
            match exit_result {
                Ok(_) => break,
                Err(error) => {
                    coordinator_tx
                        .send(CoordinatorMessage::Shutdown)
                        .expect("Error sending shutdown message");
                    panic!("Error while running nodejs proxy server: {}", error);
                }
            }
        }
    });

    Ok(SetupProxyThreadsJoin {
        listener_join,
        monitor_join,
    })
}

/// Which phase of the setup is to be run.
///
/// TODO: confirm is "Phase" the correct terminology here?
#[derive(Debug, Clone, Copy)]
pub enum SetupPhase {
    Development,
    Inner,
    Outer,
    Universal,
}

/// Configuration for the [run_coordinator()] function to run
/// `aleo-setup-coordinator` rocket server.
#[derive(Debug)]
struct CoordinatorConfig {
    /// The location of the `aleo-setup-coordinator` repository.
    pub crate_dir: PathBuf,
    /// The location of the `aleo-setup-coordinator` binary (including
    /// the binary name).
    pub setup_coordinator_bin: PathBuf,
    /// What phase of the setup ceremony to run.
    pub phase: SetupPhase,
}

impl Display for SetupPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SetupPhase::Development => "development",
            SetupPhase::Inner => "inner",
            SetupPhase::Outer => "outer",
            SetupPhase::Universal => "universal",
        };

        write!(f, "{}", s)
    }
}

/// Run the `aleo-setup-coordinator`. This will first wait for the
/// nodejs proxy to start (which will publish a
/// [CoordinatorMessage::CoordinatorProxyReady]).
///
/// TODO: return a thread join handle.
/// TODO: make a monitor thread (like in the proxy).
fn run_coordinator(
    config: CoordinatorConfig,
    coordinator_tx: Sender<CoordinatorMessage>,
    coordinator_rx: Receiver<CoordinatorMessage>,
) -> eyre::Result<()> {
    let span = tracing::error_span!("coordinator");
    let _guard = span.enter();

    tracing::info!("Setup coordinator waiting for nodejs proxy to start.");

    // Wait for the coordinator proxy to report that it's ready.
    for message in coordinator_rx.recv() {
        match message {
            CoordinatorMessage::CoordinatorProxyReady => break,
            CoordinatorMessage::Shutdown => return Ok(()),
            _ => {
                tracing::error!("Unexpected message: {:?}", message);
            }
        }
    }

    tracing::info!("Starting setup coordinator.");

    Exec::cmd(std::fs::canonicalize(config.setup_coordinator_bin)?)
        .cwd(config.crate_dir)
        .arg(config.phase.to_string())
        .join()
        .map_err(eyre::Error::from)
        .and_then(parse_exit_status)?;

    // TODO: wait for the `Rocket has launched from` message on
    // STDOUT, just like how it is implemented in
    // run_coordinator_proxy(), then send the
    // `CoordinatorMessage::CoordinatorReady` to notify the verifier
    // and the participants that they can start.

    Ok(())
}

/// The directory that the `aleo-setup-coordinator` repository is
/// cloned to.
const SETUP_COORDINATOR_DIR: &str = "aleo-setup-coordinator";

/// The directory that the `aleo-setup` repository is cloned to.
const SETUP_DIR: &str = "aleo-setup";

/// The main method of the test, which runs the test. In the future
/// this may accept command line arguments to configure how the test
/// is run.
fn main() -> eyre::Result<()> {
    setup_reporting()?;

    // Install a specific version of the rust toolchain needed to be
    // able to compile `aleo-setup`.
    let rust_1_47_nightly = RustToolchain::Specific("nightly-2020-08-15".to_string());
    install_rust_toolchain(&rust_1_47_nightly)?;

    // Clone the git repos for `aleo-setup` and
    // `aleo-setup-coordinator`.
    //
    // **NOTE: currently I am commenting out these lines during
    // development of this test**
    //
    // TODO: implement a command line argument that will ignore this
    // step if the repos are already cloned, for development purposes.
    // In the actual test it's probably good for this to fail if it's
    // trying to overwrite a previous test, it should be starting
    // clean.
    // get_git_repository(
    //     "https://github.com/AleoHQ/aleo-setup-coordinator",
    //     SETUP_COORDINATOR_DIR,
    // )?;
    // get_git_repository("https://github.com/AleoHQ/aleo-setup", SETUP_DIR)?;

    // Build the setup coordinator Rust project.
    let coordinator_output_dir = build_rust_crate(SETUP_COORDINATOR_DIR, &rust_1_47_nightly)?;
    let coordinator_bin = coordinator_output_dir.join("aleo-setup-coordinator");

    // Install the dependencies for the setup coordinator nodejs proxy.
    npm_install(SETUP_COORDINATOR_DIR)?;

    // Create some mpmc channels for communicating between the various
    // components that run during the integration test.
    let (coordinator_tx, coordinator_rx) = flume::unbounded::<CoordinatorMessage>();

    // Run the nodejs proxy server for the coordinator.
    let setup_proxy_join = run_coordinator_proxy(
        SETUP_COORDINATOR_DIR,
        coordinator_tx.clone(),
        coordinator_rx.clone(),
    )?;

    let coordinator_config = CoordinatorConfig {
        crate_dir: PathBuf::from_str(SETUP_COORDINATOR_DIR)?,
        setup_coordinator_bin: coordinator_bin,
        phase: SetupPhase::Development,
    };

    // Run the coordinator (which will first wait for the proxy to start).
    run_coordinator(
        coordinator_config,
        coordinator_tx.clone(),
        coordinator_rx.clone(),
    )?;

    // TODO: start the `setup1-verifier` and `setup1-contributor`.

    tracing::debug!("Test complete, waiting for the other threads to shutdown safely");

    // Tell the other threads to shutdown, safely terminating their
    // child processes.
    coordinator_tx
        .send(CoordinatorMessage::Shutdown)
        .expect("unable to send message");

    // Wait for the setup proxy threads to close after being told to shut down.
    setup_proxy_join
        .join()
        .expect("error while joining setup proxy threads");

    Ok(())
}

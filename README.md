# Aleo Setup Integration Test

This repository contains the integration test for [aleo-setup](https://github.com/AleoHQ/aleo-setup) (`setup1-contributor` and `setup1-verifier` binaries), and the [aleo-setup-coordinator](https://github.com/AleoHQ/aleo-setup-coordinator/).

## Prerequisites

Before running, you need to have the following installed:

+ [`rustup`](https://rustup.rs/), and the required linker for you system. (on Ubuntu this involves installing `build-essential` most likely).
+ openssl development headers (`libssl-dev` and `pkg-config` on Ubuntu).
+ Python 3 with `pip` (if you want to use the [`aleo-setup-state-monitor`](https://github.com/AleoHQ/aleo-setup-state-monitor)).

Install the stable version of rust to compile this integration testing software:

```bash
rustup install stable
```

## Building/Running

### Single

You can run a single simple integration test directly from the command line:

```bash
cargo run
```

Available command line options can be found:

```bash
cargo run -- --help
```

### Multi

You can run the scripted development integration tests with the following `multi` sub-command:

```bash
cargo run -- multi dev-tests.ron
```

See the other `*-tests.ron` files for tests for each of the setups `development`, `inner`, `outer`, and `universal` (not yet implemented).

## Configuration Format

See [example-config.ron](./example-config.ron) in the repository root for an example of the configuration format. They use the [Rusty Object Notation (RON)](https://github.com/ron-rs/ron) format, there are editor extensions available. This format was chosen because it allows structured/nested data (like JSON) but also allows comments and looser formatting for handwritten files (like TOML).

### Using Local Repositories

If you wish to use repositories checked out on your local computer you can supply the following configuration options (replacing the existing ones) at the beginning of the `ron` test file:

```ron
aleo_setup_repo: (
    type: "Local",
    dir: "../aleo-setup",
),
aleo_setup_coordinator_repo: (
    type: "Local",
    dir: "../aleo-setup-coordinator",
),
aleo_setup_state_monitor_repo: (
    type: "Local",
    dir: "../aleo-setup-state-monitor",
),
```

`dir` is the relative path to where you have the git repository checked out.

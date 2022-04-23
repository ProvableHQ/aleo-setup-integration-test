//! This module contains functions for running multiple integration
//! tests.

use serde::Deserialize;

use crate::{test::TestRoundSpec, Environment};

/// Specification for multiple tests to be performed. Will be
/// deserialized from a ron file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Specification {
    /// Specifications for the individual tests.
    pub tests: Vec<SingleTestSpec>,
}

pub type TestId = String;

/// Options for each individual test in the [Specification]'s `tests`
/// field.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct SingleTestSpec {
    /// Id for the individual test.
    pub id: TestId,

    /// Number of verifier participants for the test.
    pub verifiers: u8,

    /// (Optional) Number of replacement contributors for the test.
    /// Default: 0
    #[serde(default = "default_replacement_contributors")]
    pub replacement_contributors: u8,

    /// What environment to use for the setup.
    pub environment: Environment,

    /// (Optional) Time limit for this individual test (in seconds).
    /// Exceeding this will cause the test to fail. If set to
    /// `None`  then there is no time limit. Default: `None`
    #[serde(default)]
    pub timout: Option<u64>,

    /// (Optional) Whether to skip running this test. Default:
    /// `false`.
    #[serde(default = "skip_default")]
    pub skip: bool,

    /// Configure the tests performed for each round of the ceremony.
    pub rounds: Vec<TestRoundSpec>,
}

/// Default value for [SingleTestOptions::replacement_contributors].
fn default_replacement_contributors() -> u8 {
    0
}

/// Default value for [SingleTestOptions::skip].
fn skip_default() -> bool {
    false
}

#[cfg(test)]
mod test {
    use super::Specification;

    /// Test deserializing `example-config.ron` to [Specification].
    #[test]
    fn test_deserialize_example() {
        let example_string = std::fs::read_to_string("example-specification.ron")
            .expect("Error while reading example-specification.ron file");
        let _example: Specification =
            ron::from_str(&example_string).expect("Error while deserializing example-config.ron");
    }
}

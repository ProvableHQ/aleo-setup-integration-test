//! Module to handle running the host for the browser contributor's frontend code using `npm`.

use crate::Environment;

pub struct FrontendConfiguration {
    /// The setup we are going to run.
    setup: Environment,
}

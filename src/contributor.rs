use crate::AleoPublicKey;

/// A reference to a contributor in the ceremony.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ContributorRef {
    /// Public aleo address
    pub address: AleoPublicKey,
}

impl std::fmt::Display for ContributorRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.address.fmt(f)
    }
}

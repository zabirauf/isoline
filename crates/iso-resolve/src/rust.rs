//! Owned by its language agent (see CONTRACTS.md).

use iso_core::ImportFact;

use crate::{RepoFiles, Resolution, Resolver};

pub struct RustResolver {
    pub repo: RepoFiles,
}

impl RustResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        RustResolver { repo: repo.clone() }
    }
}

impl Resolver for RustResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        crate::generic_relative(from_rel, &import.specifier, &self.repo)
    }
}

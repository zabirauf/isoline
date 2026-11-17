//! Owned by its language agent (see CONTRACTS.md).

use iso_core::ImportFact;

use crate::{RepoFiles, Resolution, Resolver};

pub struct LinkResolver {
    pub repo: RepoFiles,
}

impl LinkResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        LinkResolver { repo: repo.clone() }
    }
}

impl Resolver for LinkResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        crate::generic_relative(from_rel, &import.specifier, &self.repo)
    }
}

//! Owned by its language agent (see CONTRACTS.md).

use iso_core::ImportFact;

use crate::{RepoFiles, Resolution, Resolver};

pub struct PythonResolver {
    pub repo: RepoFiles,
}

impl PythonResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        PythonResolver { repo: repo.clone() }
    }
}

impl Resolver for PythonResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        crate::generic_relative(from_rel, &import.specifier, &self.repo)
    }
}

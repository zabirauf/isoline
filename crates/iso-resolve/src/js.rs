//! Owned by its language agent (see CONTRACTS.md).

use iso_core::ImportFact;

use crate::{RepoFiles, Resolution, Resolver};

pub struct JsResolver {
    pub repo: RepoFiles,
}

impl JsResolver {
    pub fn new(repo: &RepoFiles) -> Self {
        JsResolver { repo: repo.clone() }
    }
}

impl Resolver for JsResolver {
    fn resolve(&self, from_rel: &str, import: &ImportFact) -> Resolution {
        crate::generic_relative(from_rel, &import.specifier, &self.repo)
    }
}

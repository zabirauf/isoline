//! Link resolution for HTML (`<script src>`, `<link href>`) and CSS (`@import`). Owned by
//! workstream D (see CONTRACTS.md).
//!
//! - `?query` and `#fragment` are dropped and `%XX` escapes decoded.
//! - `http(s)://host/…` and `//host/…` are `External(host)`; other schemes (`data:`,
//!   `mailto:`) are `External(scheme)`; webpack-style `~pkg/x.css` is `External(pkg)`.
//! - Root-relative `/x.css` is tried at the repo root, then in the linking file's directory
//!   and each of its ancestors (nearest first), for sites served from a subdirectory.
//! - Anything else is relative to the linking file.
//! - Template syntax (`{{ }}`, `{% %}`, `<% %>`, `${}`) and misses are `Unresolved`.

use iso_core::ImportFact;

use crate::{RepoFiles, Resolution, Resolver, join_rel, parent_dir};

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
        let spec = import.specifier.trim();
        if spec.is_empty() || ["{{", "{%", "<%", "${"].iter().any(|t| spec.contains(t)) {
            return Resolution::Unresolved;
        }
        if let Some(rest) = spec.strip_prefix("//") {
            return Resolution::External(host(rest));
        }
        if let Some((scheme, rest)) = spec.split_once(':')
            && !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            let lower = scheme.to_ascii_lowercase();
            if lower == "http" || lower == "https" {
                return Resolution::External(host(rest.trim_start_matches('/')));
            }
            return Resolution::External(lower);
        }
        if let Some(pkg) = spec.strip_prefix('~') {
            let name = pkg.trim_start_matches('/');
            let first = if name.starts_with('@') {
                name.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
            } else {
                name.split('/').next().unwrap_or(name).to_string()
            };
            return Resolution::External(first);
        }
        let path = spec.split(['?', '#']).next().unwrap_or(spec);
        let path = percent_decode(path);
        if path.is_empty() {
            return Resolution::Unresolved;
        }
        if let Some(abs) = path.strip_prefix('/') {
            // Root first, then the file's own directory and its ancestors.
            let mut candidates = vec![join_rel("", abs)];
            let mut dir = parent_dir(from_rel);
            while !dir.is_empty() {
                candidates.push(join_rel(dir, abs));
                dir = parent_dir(dir);
            }
            return candidates
                .into_iter()
                .find(|c| self.repo.contains(c))
                .map(Resolution::File)
                .unwrap_or(Resolution::Unresolved);
        }
        let joined = join_rel(parent_dir(from_rel), &path);
        if self.repo.contains(&joined) {
            Resolution::File(joined)
        } else {
            Resolution::Unresolved
        }
    }
}

fn host(rest: &str) -> String {
    let h = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let h = h.rsplit('@').next().unwrap_or(h);
    h.to_ascii_lowercase()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

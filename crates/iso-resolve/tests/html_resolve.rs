//! Link resolver (HTML and CSS) against `fixtures/html/site`.

use iso_core::ImportKind;
use iso_resolve::html::LinkResolver;
use iso_resolve::{Resolution, Resolver, Resolvers};

#[path = "rust_test_support.rs"]
mod support;
use support::{check_resolutions, fixture, import, repo_at};

#[test]
fn html_site_fixture() {
    let root = fixture("html", "site");
    let r = LinkResolver::new(&repo_at(&root));
    assert!(check_resolutions(&root, &r) >= 10);
}

#[test]
fn html_css_dispatch_uses_link_resolver() {
    let root = fixture("html", "site");
    let all = Resolvers::new(&repo_at(&root));
    let got = all.resolve(
        iso_core::Language::Css,
        "css/site.css",
        &import("base.css", &[], ImportKind::Link),
    );
    assert_eq!(got, Resolution::File("css/base.css".into()));
    let r = LinkResolver::new(&repo_at(&root));
    assert_eq!(
        r.resolve(
            "index.html",
            &import("~@scope/pkg/x.css", &[], ImportKind::Link)
        ),
        Resolution::External("@scope/pkg".into())
    );
    assert_eq!(
        r.resolve(
            "index.html",
            &import("css/site%2Ecss", &[], ImportKind::Link)
        ),
        Resolution::File("css/site.css".into())
    );
}

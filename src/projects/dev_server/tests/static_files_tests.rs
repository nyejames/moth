//! Tests for static path resolution, content-type mapping, and HTML snippet injection.

use super::{
    ResolvedRequest, ResolvedRequestKind, content_type_for_path, inject_dev_client, resolve_request,
};
use crate::projects::routing::{HtmlSiteConfig, PageUrlStyle};
use std::fs;
use std::path::Path;

fn site_config(page_url_style: PageUrlStyle, redirect_index_html: bool) -> HtmlSiteConfig {
    HtmlSiteConfig {
        origin: String::from("/"),
        page_url_style,
        redirect_index_html,
    }
}

#[test]
fn injection_precedes_page_scripts_and_happens_once() {
    for html in [
        "<html><head><script>bootstrap()</script></head><body>Hello</body></html>",
        "<html><head><!-- <script>unused()</script> --><script>bootstrap()</script></head><body>Hello</body></html>",
        "<html><HEAD><title>Hello</title></HEAD><body><script>bootstrap()</script></body></html>",
        "<html><body><script>bootstrap()</script></body></html>",
        "<html><body><h1>Hello</h1></body></html>",
    ] {
        let injected = inject_dev_client(html, "/", 7, "docs/index.html");
        assert_eq!(injected.matches("new EventSource(").count(), 1);
        let hook = injected
            .find("globalThis.__moth_record_entry_failure")
            .expect("hook installed");
        if let Some(bootstrap) = injected.find("bootstrap()") {
            assert!(
                hook < bootstrap,
                "hooks must exist before classic page bootstrap"
            );
        }
        assert_eq!(
            inject_dev_client(&injected, "/", 7, "docs/index.html"),
            injected
        );
    }
}

#[test]
fn known_entries_are_exact_html_files_inside_the_output() {
    let output = tempfile::tempdir().expect("output directory");
    fs::create_dir(output.path().join("docs")).expect("page directory");
    fs::write(output.path().join("docs/index.html"), "page").expect("page file");
    fs::write(output.path().join("app.js"), "script").expect("script file");
    assert!(super::is_known_entry("docs/index.html", output.path()));
    for invalid in [
        "",
        "/docs/index.html",
        "../index.html",
        "docs/./index.html",
        "docs//index.html",
        "docs\\index.html",
        "missing.html",
        "app.js",
    ] {
        assert!(!super::is_known_entry(invalid, output.path()), "{invalid}");
    }
}

#[cfg(unix)]
#[test]
fn known_entries_reject_symlinks_outside_the_output() {
    let output = tempfile::tempdir().expect("output directory");
    let outside = tempfile::tempdir().expect("external directory");
    let external_page = outside.path().join("index.html");
    fs::write(&external_page, "external page").expect("external page");
    std::os::unix::fs::symlink(external_page, output.path().join("index.html"))
        .expect("page symlink");
    assert!(!super::is_known_entry("index.html", output.path()));
}

#[test]
fn content_type_map_covers_common_extensions() {
    assert_eq!(
        content_type_for_path(Path::new("index.html")),
        "text/html; charset=utf-8"
    );
    assert_eq!(
        content_type_for_path(Path::new("bundle.js")),
        "application/javascript; charset=utf-8"
    );
    assert_eq!(content_type_for_path(Path::new("image.png")), "image/png");
}

#[test]
fn resolve_path_rejects_traversal() {
    let output_dir = Path::new("/tmp/project/dev");
    let resolved = resolve_request(
        "/../secret.txt",
        None,
        output_dir,
        Some(Path::new("index.html")),
        HtmlSiteConfig::default(),
    );
    assert_eq!(resolved, ResolvedRequest::InvalidPath);
}

#[test]
fn root_uses_entry_page_when_available() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(&output_dir).expect("should create output dir");
    fs::write(output_dir.join("index.html"), "<h1>home</h1>").expect("should write root page");

    let resolved = resolve_request(
        "/",
        None,
        &output_dir,
        Some(Path::new("index.html")),
        HtmlSiteConfig::default(),
    );

    assert_eq!(
        resolved,
        ResolvedRequest::File {
            path: output_dir.join("index.html"),
            kind: ResolvedRequestKind::PageHtml,
        }
    );
}

#[test]
fn trailing_slash_mode_redirects_non_canonical_page_forms() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("about")).expect("should create about dir");
    fs::write(output_dir.join("about/index.html"), "<h1>about</h1>").expect("should write page");

    let cfg = site_config(PageUrlStyle::TrailingSlash, true);

    assert_eq!(
        resolve_request(
            "/about",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::Redirect {
            location: String::from("/about/"),
        }
    );
    assert_eq!(
        resolve_request(
            "/about/index.html",
            Some("x=1"),
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::Redirect {
            location: String::from("/about/?x=1"),
        }
    );
    assert_eq!(
        resolve_request(
            "/about/",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg
        ),
        ResolvedRequest::File {
            path: output_dir.join("about/index.html"),
            kind: ResolvedRequestKind::PageHtml,
        }
    );
}

#[test]
fn no_trailing_slash_mode_redirects_trailing_page_form() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("about")).expect("should create about dir");
    fs::write(output_dir.join("about/index.html"), "<h1>about</h1>").expect("should write page");

    let cfg = site_config(PageUrlStyle::NoTrailingSlash, true);

    assert_eq!(
        resolve_request(
            "/about/",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::Redirect {
            location: String::from("/about"),
        }
    );
    assert_eq!(
        resolve_request(
            "/about",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::File {
            path: output_dir.join("about/index.html"),
            kind: ResolvedRequestKind::PageHtml,
        }
    );
    assert_eq!(
        resolve_request(
            "/about/index.html",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg
        ),
        ResolvedRequest::Redirect {
            location: String::from("/about"),
        }
    );
}

#[test]
fn ignore_mode_serves_both_slash_forms_but_can_still_redirect_index_alias() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("about")).expect("should create about dir");
    fs::write(output_dir.join("about/index.html"), "<h1>about</h1>").expect("should write page");

    let cfg = site_config(PageUrlStyle::Ignore, true);

    assert_eq!(
        resolve_request(
            "/about",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::File {
            path: output_dir.join("about/index.html"),
            kind: ResolvedRequestKind::PageHtml,
        }
    );
    assert_eq!(
        resolve_request(
            "/about/",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::File {
            path: output_dir.join("about/index.html"),
            kind: ResolvedRequestKind::PageHtml,
        }
    );
    assert_eq!(
        resolve_request(
            "/about/index.html",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg
        ),
        ResolvedRequest::Redirect {
            location: String::from("/about/"),
        }
    );
}

#[test]
fn exact_assets_are_served_without_page_canonicalization() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("images")).expect("should create images dir");
    fs::write(output_dir.join("app.js"), "console.log('ok');").expect("should write js");
    fs::write(output_dir.join("images/logo.png"), [0x89, b'P', b'N', b'G'])
        .expect("should write png");
    fs::write(output_dir.join("CNAME"), "example.com").expect("should write extensionless file");

    let cfg = HtmlSiteConfig::default();

    assert_eq!(
        resolve_request(
            "/app.js",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::File {
            path: output_dir.join("app.js"),
            kind: ResolvedRequestKind::Asset,
        }
    );
    assert_eq!(
        resolve_request(
            "/app.js/",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::NotFound
    );
    assert_eq!(
        resolve_request(
            "/images/logo",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg.clone()
        ),
        ResolvedRequest::NotFound
    );
    assert_eq!(
        resolve_request(
            "/CNAME",
            None,
            &output_dir,
            Some(Path::new("index.html")),
            cfg
        ),
        ResolvedRequest::File {
            path: output_dir.join("CNAME"),
            kind: ResolvedRequestKind::Asset,
        }
    );
}

#[test]
fn origin_aware_resolution_and_redirects() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("docs")).expect("should create docs dir");
    fs::write(output_dir.join("docs/index.html"), "<h1>docs</h1>").expect("should write page");
    fs::write(output_dir.join("site.css"), "body {}").expect("should write asset");

    let cfg = HtmlSiteConfig {
        origin: String::from("/moth"),
        page_url_style: PageUrlStyle::TrailingSlash,
        redirect_index_html: true,
    };

    // 1. Request inside origin - page
    assert_eq!(
        resolve_request("/docs/", None, &output_dir, None, cfg.clone()),
        ResolvedRequest::File {
            path: output_dir.join("docs/index.html"),
            kind: ResolvedRequestKind::PageHtml,
        }
    );

    // 2. Request inside origin - redirect to canonical slash
    assert_eq!(
        resolve_request("/docs", None, &output_dir, None, cfg.clone()),
        ResolvedRequest::Redirect {
            location: String::from("/moth/docs/"),
        }
    );

    // 3. Request inside origin - index.html alias redirect
    assert_eq!(
        resolve_request("/docs/index.html", None, &output_dir, None, cfg.clone()),
        ResolvedRequest::Redirect {
            location: String::from("/moth/docs/"),
        }
    );

    // 4. Request inside origin - asset
    assert_eq!(
        resolve_request("/site.css", None, &output_dir, None, cfg.clone()),
        ResolvedRequest::File {
            path: output_dir.join("site.css"),
            kind: ResolvedRequestKind::Asset,
        }
    );
}

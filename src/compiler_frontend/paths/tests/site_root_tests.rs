//! Unit tests for site-root URL rendering.
//!
//! Project configuration already rejects an origin that is empty, lacks a leading `/` or carries a
//! trailing `/` beyond the bare root, so these cover the accepted spellings plus the degenerate
//! inputs the frontend default and other builders can still supply.

use crate::compiler_frontend::paths::site_root::render_site_root_url;

#[test]
fn an_unset_origin_renders_the_bare_site_root() {
    assert_eq!(render_site_root_url("/"), "/");
}

#[test]
fn a_configured_origin_renders_with_one_trailing_slash() {
    assert_eq!(render_site_root_url("/moth"), "/moth/");
}

#[test]
fn a_nested_origin_keeps_every_prefix_segment() {
    assert_eq!(render_site_root_url("/moth/docs"), "/moth/docs/");
}

/// The rendered site root always ends in `/`, so `[@/]docs/` composes by concatenation rather than
/// by a separator rule at each authored use.
#[test]
fn a_rendered_site_root_composes_with_an_authored_suffix() {
    assert_eq!(
        format!("{}docs/", render_site_root_url("/moth")),
        "/moth/docs/"
    );
    assert_eq!(format!("{}docs/", render_site_root_url("/")), "/docs/");
}

#[test]
fn a_degenerate_origin_still_renders_the_bare_site_root() {
    assert_eq!(render_site_root_url(""), "/");
    assert_eq!(render_site_root_url("///"), "/");
}

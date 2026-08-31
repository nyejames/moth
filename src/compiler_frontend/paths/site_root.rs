//! Site-root URL rendering.
//!
//! WHAT: turns the configured project origin into the URL that the bare site-root spelling `@/`
//! renders.
//! WHY: a site-root URL and a resource URL answer different questions, and keeping them apart is
//! what makes both correct. A resource URL is written relative to the artefact that observes it and
//! never carries the origin. A site-root URL addresses a route rather than a file the build emits,
//! so it is absolute and always carries the origin.
//!
//! The site root names no file. It has no resource origin, owner, byte source or watch interest,
//! and is never checked, copied, hashed, rewritten or included in a resource union.

/// Renders the site-root URL for one configured project origin.
///
/// The result always ends in `/`, so an authored suffix such as `[@/]docs/` composes by
/// concatenation. Project configuration spells an unset origin `/`, which renders as the bare
/// site root.
pub(crate) fn render_site_root_url(origin: &str) -> String {
    let prefix = origin.trim_end_matches('/');

    if prefix.is_empty() {
        return String::from("/");
    }

    format!("{prefix}/")
}

#[cfg(test)]
#[path = "tests/site_root_tests.rs"]
mod tests;

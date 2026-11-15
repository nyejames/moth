//! HTML project builder-owned binding packages.
//!
//! WHAT: houses built-in JS-backed packages such as `@builder/web/canvas` that the HTML builder
//!       registers directly as virtual packages with runtime asset metadata.
//! WHY: builder-owned binding packages share the same parser and emission path as project-local
//!      `.js` imports, but their package paths and registration are controlled by Rust code.

pub mod web;

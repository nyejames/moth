//! Build-system-aware path string formatting for compile-time path values.
//!
//! WHAT: formats resolved `CompileTimePath` values into public string
//! representations, applying the origin prefix and output style policies.
//!
//! WHY: path-to-string coercion rules (origin prefix, trailing slash,
//! relative preservation) belong in one shared module so all builders
//! consume consistent output without reimplementing the rules.

use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePath, CompileTimePathBase, CompileTimePathKind,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Output path separator style.
///
/// `Portable` uses forward slashes and is the default for web output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPathStyle {
    Portable,
}

/// Configuration for path string formatting.
///
/// WHAT: carries the origin prefix and output style so formatting is deterministic.
/// WHY: builders must agree on one source of truth for these policies.
#[derive(Debug, Clone)]
pub struct PathStringFormatConfig {
    /// The `origin` config key value (e.g. `"/moth"`).
    /// A bare `"/"` means no prefix is added.
    pub origin: String,
    /// Separator style for the formatted output.
    pub output_style: OutputPathStyle,
}

impl Default for PathStringFormatConfig {
    fn default() -> Self {
        Self {
            origin: String::from("/"),
            output_style: OutputPathStyle::Portable,
        }
    }
}

/// WHAT: formats a compile-time path into a public string representation.
/// WHY: this is the single place where the origin prefix, trailing slash, and
/// relative-path preservation rules are applied.
///
/// Rules:
/// - Relative paths (`RelativeToFile`) stay relative; no origin is applied.
/// - Root-based paths (`SourcePackageRoot`, `EntryRoot`) get a leading `/`
///   and are prefixed with origin when origin is not `"/"`.
/// - Directory paths get a trailing `/`.
/// - The `Portable` output style always uses forward slashes.
pub fn format_compile_time_path(
    path: &CompileTimePath,
    config: &PathStringFormatConfig,
    string_table: &StringTable,
) -> String {
    let raw = render_public_path(&path.public_path, &path.base, string_table);

    let with_trailing = match path.kind {
        CompileTimePathKind::Directory => ensure_trailing_slash(&raw),
        CompileTimePathKind::File => raw,
    };

    let formatted = match path.base {
        CompileTimePathBase::RelativeToFile => with_trailing,
        CompileTimePathBase::SourcePackageRoot | CompileTimePathBase::EntryRoot => {
            apply_origin(&with_trailing, &config.origin)
        }
    };

    match config.output_style {
        OutputPathStyle::Portable => formatted.replace('\\', "/"),
    }
}

/// Renders the public path components into a string with appropriate leading
/// characters based on the path base.
fn render_public_path(
    public_path: &InternedPath,
    base: &CompileTimePathBase,
    string_table: &StringTable,
) -> String {
    let portable = public_path.to_portable_string(string_table);

    match base {
        CompileTimePathBase::RelativeToFile => {
            // Relative paths keep their form as-is (e.g. "./images/logo.png").
            portable
        }
        CompileTimePathBase::SourcePackageRoot | CompileTimePathBase::EntryRoot => {
            // Non-relative paths become absolute site paths: "/assets/logo.png".
            // An empty public path here is the Moth public-root literal (`@/`),
            // which renders as "/" before origin is applied.
            if portable.starts_with('/') {
                portable
            } else {
                format!("/{portable}")
            }
        }
    }
}

/// Ensures the string ends with exactly one `/`.
fn ensure_trailing_slash(s: &str) -> String {
    if s.ends_with('/') {
        s.to_owned()
    } else {
        format!("{s}/")
    }
}

/// Applies the origin prefix to an absolute site path.
fn apply_origin(site_path: &str, origin: &str) -> String {
    if origin == "/" {
        return site_path.to_owned();
    }

    // Origin is applied exactly once at the public formatting boundary.
    // Callers must not prepend origin to already formatted path strings.

    // origin is like "/moth", site_path is like "/assets/logo.png"
    // result: "/moth/assets/logo.png"
    format!("{origin}{site_path}")
}

#[cfg(test)]
#[path = "tests/path_format_tests.rs"]
mod tests;

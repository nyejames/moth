//! Semantic rendered-path capture for builder-visible output usage facts.
//!
//! WHAT: records which compile-time paths were rendered into HTML/template output, how they
//! resolved, and where that render happened.
//!
//! WHY: builders need semantic path provenance after path formatting has converted a value into
//! text, but output placement policy must stay builder-owned rather than frontend-owned.

use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePath, CompileTimePathBase, CompileTimePathKind, CompileTimePathResolutionError,
};
use crate::compiler_frontend::paths::path_format::{
    PathStringFormatConfig, format_compile_time_path,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::path::{Path, PathBuf};

/// Builder-visible semantic fact for one compile-time path rendered into output text.
///
/// WHAT: preserves the authored path, resolved filesystem target, public path, resolution base,
/// target kind, and precise render site.
///
/// WHY: builders need stable semantic inputs for tracked-asset planning after the frontend has
/// already rendered human/browser-visible path strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPathUsage {
    /// Path exactly as authored in source.
    pub source_path: InternedPath,
    /// Fully resolved source-side filesystem target.
    pub filesystem_path: PathBuf,
    /// Public-facing resolved path before builder-owned emission policy.
    pub public_path: InternedPath,
    /// Resolution base (`RelativeToFile`, `SourcePackageRoot`, or `EntryRoot`).
    pub base: CompileTimePathBase,
    /// Whether the resolved target is a file or directory.
    pub kind: CompileTimePathKind,
    /// Real source file that rendered this path.
    pub source_file_scope: InternedPath,
    /// Render location used for builder diagnostics and warnings.
    pub render_location: SourceLocation,
}

/// Small capture result used at rendered output boundaries.
///
/// WHAT: returns the formatted text needed by the current frontend call site plus the semantic
/// usage records builders will later consume.
///
/// WHY: v1 still renders path strings eagerly in the frontend, but captured semantics let builders
/// plan tracked assets without reconstructing intent from flat strings.
///
/// Planned(html-assets): if tracked-asset URL rewriting moves later in the pipeline, replace or
/// wrap this eager string result with a deferred rendered-path representation instead of adding
/// builder heuristics around already-rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRenderedPaths {
    pub rendered_text: String,
    pub usages: Vec<RenderedPathUsage>,
}

/// Resolve compile-time path tokens and capture their rendered-output usage facts in one step.
///
/// WHAT: normalizes resolution, rendered text formatting, and semantic usage recording behind one
/// helper for rendered output boundaries.
///
/// WHY: path-to-string output must stay consistent across call sites, and builders must not lose
/// provenance when the frontend eagerly formats text.
pub(crate) fn resolve_compile_time_path_for_rendered_output(
    path: &InternedPath,
    project_path_resolver: &ProjectPathResolver,
    declaring_file: &Path,
    source_file_scope: &InternedPath,
    render_location: &SourceLocation,
    path_format_config: &PathStringFormatConfig,
    string_table: &mut StringTable,
) -> Result<(CompileTimePath, RecordedRenderedPaths), CompileTimePathResolutionError> {
    let resolved =
        project_path_resolver.resolve_compile_time_path(path, declaring_file, string_table)?;
    let recorded = record_compile_time_path_for_rendered_output(
        &resolved,
        source_file_scope,
        render_location,
        path_format_config,
        string_table,
    );

    Ok((resolved, recorded))
}

/// Capture rendered-output facts for an already-resolved compile-time path.
///
/// WHAT: keeps generic rendered-output coercion paths aligned with template-head rendering when a
/// caller already holds the canonical `CompileTimePath`.
///
/// WHY: every rendered compile-time path-to-string boundary used for HTML/template output must
/// record usage facts consistently in v1.
pub(crate) fn record_compile_time_path_for_rendered_output(
    path: &CompileTimePath,
    source_file_scope: &InternedPath,
    render_location: &SourceLocation,
    path_format_config: &PathStringFormatConfig,
    string_table: &StringTable,
) -> RecordedRenderedPaths {
    let usage = RenderedPathUsage {
        source_path: path.source_path.clone(),
        filesystem_path: path.filesystem_path.clone(),
        public_path: path.public_path.clone(),
        base: path.base.clone(),
        kind: path.kind.clone(),
        source_file_scope: source_file_scope.clone(),
        render_location: render_location.clone(),
    };

    RecordedRenderedPaths {
        rendered_text: format_compile_time_path(path, path_format_config, string_table),
        usages: vec![usage],
    }
}

#[cfg(test)]
#[path = "tests/rendered_path_usage_tests.rs"]
mod tests;

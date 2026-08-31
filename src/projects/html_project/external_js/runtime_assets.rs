//! Runtime asset emission for provider-backed external JavaScript modules.
//!
//! WHAT: turns module-carried JS runtime asset metadata into ordinary HTML builder outputs.
//! WHY: external JS files are backend artifacts, not frontend source files. Keeping their
//!      output naming and passthrough emission here keeps `HtmlProjectBuilder` focused on
//!      orchestration while preserving a single output path policy for JS assets.

use crate::build_system::build::{FileKind, OutputFile};
use crate::build_system::output::validate_relative_output_path;
use crate::build_system::utils::file_error_messages;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::path_identity::{
    sanitized_path_stem, stable_path_hash_hex,
};
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Emits all provider-backed JS runtime assets from a pre-built emission plan.
///
/// WHAT: reads each canonical JS source once and produces a `FileKind::Js` output under
/// `_moth/js/`.
/// WHY: the plan already deduplicated by canonical source path, so this function only
///      handles output-path validation, conflict checks, and filesystem reads.
pub(crate) fn emit_external_js_runtime_assets(
    plan: &HtmlExternalRuntimeEmissionPlan,
    occupied_output_paths: &mut HashSet<PathBuf>,
    string_table: &mut StringTable,
) -> Result<Vec<OutputFile>, CompilerMessages> {
    let mut output_files = Vec::with_capacity(plan.js_assets().len());

    for asset in plan.js_assets().values() {
        let output_path = js_runtime_asset_output_path(&asset.canonical_source_path);
        validate_relative_output_path(&output_path, string_table)?;

        if !occupied_output_paths.insert(output_path.clone()) {
            return Err(external_js_asset_conflicts_with_existing_output_error(
                &asset.canonical_source_path,
                &output_path,
                string_table,
            ));
        }

        let content = fs::read_to_string(&asset.canonical_source_path).map_err(|error| {
            file_error_messages(
                &asset.canonical_source_path,
                format!(
                    "Failed to read external JS asset '{}': {error}",
                    asset.canonical_source_path.display()
                ),
                string_table,
            )
        })?;

        output_files.push(OutputFile::new(output_path, FileKind::Js(content)));
    }

    Ok(output_files)
}

fn external_js_asset_conflicts_with_existing_output_error(
    source_path: &Path,
    output_path: &Path,
    string_table: &StringTable,
) -> CompilerMessages {
    let message = format!(
        "External JS asset '{}' would emit to '{}' which conflicts with an existing output.",
        source_path.display(),
        output_path.display()
    );
    CompilerMessages::from_error(CompilerError::compiler_error(message), string_table.clone())
}

/// Generate a deterministic, collision-resistant output path for a JS runtime asset.
///
/// WHAT: produces a stable relative path under `_moth/js/` using the source file stem
/// plus a stable hash of the canonical source path.
/// WHY: same-named JS files in different directories must not collide, and output names
/// should remain stable across dev rebuilds for caching/debugging.
pub(crate) fn js_runtime_asset_output_path(canonical_source_path: &Path) -> PathBuf {
    let safe_stem = sanitized_path_stem(canonical_source_path, "asset");
    let hash = stable_path_hash_hex(canonical_source_path);
    PathBuf::from("_moth/js").join(format!("{safe_stem}-{hash}.js"))
}

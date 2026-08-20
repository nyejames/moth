//! Build-level plan for external JS runtime asset and module emission.
//!
//! WHAT: collects all JS runtime assets and required runtime module specifiers from
//!       a compiled module slice in a single deterministic pass.
//! WHY: avoids repeated scans of entry-owned runtime import unions across separate emission
//!      helpers, and gives `HtmlProjectBuilder` a named build-level plan step
//!      that stays separate from per-module glue generation.
//!
//! This module must not decide per-module glue or import-map content. Those remain
//! module-local concerns owned by `runtime_glue`.

use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::module_compilation::ModuleExternalImport;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Build-level emission plan for external JS runtime artifacts.
///
/// WHAT: holds the deduplicated set of JS runtime assets and required runtime module
///       specifiers discovered from all compiled modules.
/// WHY: the HTML builder can construct this once and feed it into both asset emission
///      and runtime module emission without re-scanning module metadata.
pub(crate) struct HtmlExternalRuntimeEmissionPlan {
    /// JS runtime assets keyed by canonical source path.
    ///
    /// Only `asset_kind == "js"` assets are included, preserving current backend behavior.
    js_assets: BTreeMap<PathBuf, RuntimeAssetIdentity>,

    /// Required runtime module specifiers, e.g. `"@moth/runtime"`.
    runtime_module_specifiers: BTreeSet<String>,
}

impl HtmlExternalRuntimeEmissionPlan {
    /// Build an emission plan from the exact runtime import union of each selected entry.
    ///
    /// WHAT: scans each entry import set once to collect:
    ///       - JS runtime assets by canonical source path;
    ///       - runtime module specifiers from `required_runtime_imports`.
    /// WHY: deterministic deduplication in one pass avoids redundant iteration later.
    pub(crate) fn from_import_sets<'a>(
        import_sets: impl IntoIterator<Item = &'a [ModuleExternalImport]>,
    ) -> Self {
        let mut js_assets = BTreeMap::new();
        let mut runtime_module_specifiers = BTreeSet::new();

        for external_imports in import_sets {
            for external_import in external_imports {
                if let Some(asset) = &external_import.runtime_asset
                    && asset.asset_kind == "js"
                {
                    js_assets
                        .entry(asset.canonical_source_path.clone())
                        .or_insert_with(|| asset.clone());
                }

                for runtime_import in &external_import.required_runtime_imports {
                    runtime_module_specifiers.insert(runtime_import.module_name.clone());
                }
            }
        }

        Self {
            js_assets,
            runtime_module_specifiers,
        }
    }

    pub(crate) fn js_assets(&self) -> &BTreeMap<PathBuf, RuntimeAssetIdentity> {
        &self.js_assets
    }

    pub(crate) fn runtime_module_specifiers(&self) -> &BTreeSet<String> {
        &self.runtime_module_specifiers
    }
}

#[cfg(test)]
#[path = "tests/runtime_emission_plan_tests.rs"]
mod tests;

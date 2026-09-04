//! JavaScript backend for Moth.
//!
//! This backend lowers HIR into readable JavaScript using GC semantics.
//! Borrowing and ownership are optimization concerns and therefore ignored here.

mod emitter;
mod identifiers;
mod js_calls;
mod js_expr;
mod js_function;
mod js_statement;
mod lookups;
mod output;
pub(crate) mod package_bindings;
mod reachability;
mod runtime;
mod symbols;
mod value_use;

#[cfg(test)]
#[path = "tests/test_symbol_helpers.rs"]
pub(crate) mod test_symbol_helpers;
#[cfg(test)]
mod tests;

pub(crate) use emitter::JsEmitter;
pub use emitter::lower_hir_to_js;
pub(crate) use symbols::{builtin_error_code_js_field_name, builtin_error_message_js_field_name};

use crate::backends::structural_string::StructuralStringUrlMap;
use crate::compiler_frontend::external_packages::{ExternalFunctionId, ExternalPackageRegistry};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::reachability::HirBackendSelection;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Policy controlling which HIR functions are emitted in a JS bundle.
///
/// WHAT: determines whether every HIR function is lowered or only an explicit selected set.
/// WHY: project builders need selected-only emission to avoid pulling unreachable source-backed
/// package wrappers into runtime glue or asset planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsFunctionEmissionPolicy {
    /// Emit every HIR function. This is the direct JS backend contract and test default.
    AllFunctions,

    /// Emit the exact function set selected by build-owned entry planning.
    ///
    /// WHY: lowerers consume build-owned selection instead of rediscovering entry roots.
    Selected(HirBackendSelection),
}

impl JsFunctionEmissionPolicy {
    fn includes(&self, function_id: FunctionId) -> bool {
        match self {
            Self::AllFunctions => true,
            Self::Selected(selection) => selection.contains_function(function_id),
        }
    }
}

/// Configuration for JS lowering.
#[derive(Debug, Clone)]
pub struct JsLoweringConfig {
    /// Emit human-readable formatting.
    pub pretty: bool,

    /// Emit source location comments.
    pub emit_locations: bool,

    /// Automatically invoke the module start function.
    pub auto_invoke_start: bool,

    /// Controls whether the bundle contains every HIR function or a build-selected subset.
    pub function_emission_policy: JsFunctionEmissionPolicy,

    /// External package registry for resolving backend lowering metadata.
    pub external_package_registry: Arc<ExternalPackageRegistry>,
    /// Allow provider-created ES module exports to lower through generated HTML glue.
    ///
    /// WHY: only the HTML builder can emit the matching ES module glue. Direct JS backend
    /// lowering must reject these exports unless that builder path explicitly opts in.
    pub external_module_export_glue_enabled: bool,
    /// Build-owned stable source-call symbol plan shared by every module in one entry assembly.
    pub source_function_names: Arc<HashMap<OriginFunctionId, String>>,
    /// Build-owned symbols for private executables linked into generated sidecars.
    pub module_private_function_names: Arc<HashMap<ModulePrivateExecutableIdentity, String>>,
    /// Builder-rendered text for structural strings in this physical output variant.
    pub(crate) structural_string_urls: Option<Arc<StructuralStringUrlMap>>,
    pub generated_function_names: Arc<HashMap<GeneratedFunctionIdentity, String>>,
}

impl JsLoweringConfig {
    /// Direct JS backend lowering config.
    ///
    /// WHAT: emits every HIR function with glue disabled. Used by direct JS/backend tests
    /// and any caller that needs a complete standalone JS bundle without HTML glue.
    /// WHY: the default must be all-functions emission so tests see every function;
    /// glue is disabled because no HTML builder is involved.
    pub fn direct_js(release_build: bool) -> Self {
        JsLoweringConfig {
            pretty: !release_build,
            emit_locations: false,
            auto_invoke_start: false,
            function_emission_policy: JsFunctionEmissionPolicy::AllFunctions,
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_module_export_glue_enabled: false,
            source_function_names: Arc::new(HashMap::new()),
            module_private_function_names: Arc::new(HashMap::new()),
            generated_function_names: Arc::new(HashMap::new()),
            structural_string_urls: None,
        }
    }

    /// JS-only HTML page-bundle lowering config.
    ///
    /// WHAT: emits only build-selected functions and enables ES module glue generation.
    /// WHY: unreachable source-backed package wrappers must not request runtime glue or assets.
    /// The supplied external package registry is stored directly because the HTML builder already
    /// owns it.
    pub fn html_page_bundle(
        release_build: bool,
        external_package_registry: Arc<ExternalPackageRegistry>,
        selection: HirBackendSelection,
        source_function_names: Arc<HashMap<OriginFunctionId, String>>,
        module_private_function_names: Arc<HashMap<ModulePrivateExecutableIdentity, String>>,
        generated_function_names: Arc<HashMap<GeneratedFunctionIdentity, String>>,
    ) -> Self {
        let mut config = Self::direct_js(release_build);
        config.function_emission_policy = JsFunctionEmissionPolicy::Selected(selection);
        config.external_package_registry = external_package_registry;
        config.external_module_export_glue_enabled = true;
        config.source_function_names = source_function_names;
        config.module_private_function_names = module_private_function_names;
        config.generated_function_names = generated_function_names;
        config
    }

    /// HTML-Wasm companion-JS lowering config.
    ///
    /// WHAT: emits only build-selected JS used by the Wasm bootstrap while keeping generated
    /// ES module glue disabled.
    /// WHY: this path emits bootstrap JS and Wasm artifacts, not generated glue modules.
    /// Reachable JS-backed external calls must be rejected by Wasm validation rather than
    /// silently lowered through glue that the artifact path cannot emit.
    pub(crate) fn html_wasm_companion(
        release_build: bool,
        external_package_registry: Arc<ExternalPackageRegistry>,
        selection: HirBackendSelection,
    ) -> Self {
        let mut config = Self::direct_js(release_build);
        config.function_emission_policy = JsFunctionEmissionPolicy::Selected(selection);
        config.external_package_registry = external_package_registry;
        config
    }
    /// Attach builder-rendered structural-string URLs for this lowering run.
    pub(crate) fn with_structural_string_urls(
        mut self,
        structural_string_urls: Arc<StructuralStringUrlMap>,
    ) -> Self {
        self.structural_string_urls = Some(structural_string_urls);
        self
    }
}

/// Deterministic JS identifier for a generated glue wrapper.
///
/// WHAT: maps stable external function IDs to safe wrapper function names.
/// WHY: the JS backend and the HTML glue generator must agree without duplicating naming logic.
pub(crate) fn external_module_export_glue_function_name(id: ExternalFunctionId) -> String {
    match id {
        ExternalFunctionId::Synthetic(n) => format!("__moth_glue_fn{n}"),
        other => format!("__moth_glue_{}", other.name()),
    }
}

/// Result of lowering a HIR module to JavaScript.
///
/// WHAT: carries the complete emitted JS source plus metadata needed by the HTML builder to
///       construct import maps, glue wrappers, and runtime asset plans.
#[derive(Debug, Clone)]
pub struct JsModule {
    /// Complete JS source code.
    pub source: String,
    pub function_name_by_id: HashMap<FunctionId, String>,
    /// Set of external function IDs referenced while lowering emitted JS functions.
    /// WHY: the HTML builder uses this to decide which generated glue wrappers to emit.
    pub referenced_external_functions: HashSet<ExternalFunctionId>,
}

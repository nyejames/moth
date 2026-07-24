//! JavaScript backend for Moth.
//!
//! This backend lowers HIR into readable JavaScript using GC semantics.
//! Borrowing and ownership are optimization concerns and therefore ignored here.

use std::sync::Arc;
mod emitter;
mod identifiers;
mod js_calls;
mod js_expr;
mod js_function;
mod js_statement;
mod lookups;
mod output;
mod package_bindings;
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

use crate::compiler_frontend::external_packages::{ExternalFunctionId, ExternalPackageRegistry};
use crate::compiler_frontend::hir::ids::FunctionId;
use std::collections::{HashMap, HashSet};

/// Policy controlling which HIR functions are emitted in a JS bundle.
///
/// WHAT: determines whether every HIR function is lowered or only those reachable from the
///       module entry `start` function.
/// WHY: HTML page bundles need reachable-only emission to avoid pulling in unused source-backed package
///      wrappers that would request unavailable runtime glue or assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsFunctionEmissionPolicy {
    /// Emit every HIR function. This is the direct JS backend contract and test default.
    AllFunctions,

    /// Emit only functions syntactically reachable from the module entry `start` function.
    ///
    /// WHY: HTML page bundles execute from one entry point, and unreachable source-backed package
    /// wrappers must not request runtime glue or assets.
    ReachableFromStart,
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

    /// Controls whether the bundle contains every HIR function or only entry-reachable code.
    pub function_emission_policy: JsFunctionEmissionPolicy,

    /// External package registry for resolving backend lowering metadata.
    pub external_package_registry: Arc<ExternalPackageRegistry>,
    /// Allow provider-created ES module exports to lower through generated HTML glue.
    ///
    /// WHY: only the HTML builder can emit the matching ES module glue. Direct JS backend
    /// lowering must reject these exports unless that builder path explicitly opts in.
    pub external_module_export_glue_enabled: bool,
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
        }
    }

    /// JS-only HTML page-bundle lowering config.
    ///
    /// WHAT: emits only entry-reachable functions and enables ES module glue generation.
    /// WHY: HTML page bundles execute from one entry point, so unreachable source-backed package
    /// wrappers must not request runtime glue or assets. The supplied external package
    /// registry is stored directly because the HTML builder already owns it.
    pub fn html_page_bundle(
        release_build: bool,
        external_package_registry: Arc<ExternalPackageRegistry>,
    ) -> Self {
        let mut config = Self::direct_js(release_build);
        config.function_emission_policy = JsFunctionEmissionPolicy::ReachableFromStart;
        config.external_package_registry = external_package_registry;
        config.external_module_export_glue_enabled = true;
        config
    }

    /// HTML-Wasm companion-JS lowering config.
    ///
    /// WHAT: emits only entry-reachable JS used by the Wasm bootstrap while keeping generated
    /// ES module glue disabled.
    /// WHY: this path emits bootstrap JS and Wasm artifacts, not generated glue modules.
    /// Reachable JS-backed external calls must be rejected by Wasm validation rather than
    /// silently lowered through glue that the artifact path cannot emit.
    pub(crate) fn html_wasm_companion(
        release_build: bool,
        external_package_registry: Arc<ExternalPackageRegistry>,
    ) -> Self {
        let mut config = Self::direct_js(release_build);
        config.function_emission_policy = JsFunctionEmissionPolicy::ReachableFromStart;
        config.external_package_registry = external_package_registry;
        config
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

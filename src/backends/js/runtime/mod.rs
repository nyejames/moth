//! JS runtime helper emission.
//!
//! This module emits the JS helper functions that implement Moth's runtime
//! semantics. All helper groups are declared as JS `function` declarations, which means
//! JS hoisting guarantees correct behaviour regardless of emission order.
//!
//! The top-level [`JsEmitter::emit_runtime_prelude`] only owns:
//! - helper emission order
//! - high-level comments about why these groups exist
//! - any tiny shared glue that genuinely belongs at orchestration level
//!
//! Individual helper groups live in focused submodules so semantic auditing and
//! targeted refactors are easier than with a single monolithic prelude file.

mod aliasing;
mod bindings;
mod casts;
mod choices;
mod cloning;
mod collections;
mod errors;
mod maps;
mod numeric;
mod places;
mod reactivity;
mod results;
mod strings;

use crate::backends::js::JsEmitter;

/// Describes which checked numeric runtime helper families are required by emitted JS.
///
/// WHY: arithmetic helpers, Float formatting, and Float boundary validation share the same
/// `__moth_numeric_trap` carrier wrapper, but the helper bodies themselves should stay
/// demand-driven so unrelated programs do not grow extra runtime surface.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NumericRuntimeHelperUsage {
    pub(crate) numeric_ops: bool,
    pub(crate) format_float: bool,
    pub(crate) validate_float: bool,
}

impl NumericRuntimeHelperUsage {
    pub(crate) fn any(self) -> bool {
        self.numeric_ops || self.format_float || self.validate_float
    }
}

impl<'hir> JsEmitter<'hir> {
    /// Emits the full JS runtime prelude.
    ///
    /// The JS backend preserves Moth's aliasing semantics by modeling locals and computed
    /// places as explicit reference records. The prelude is the concrete JS model for those
    /// semantics — it is not incidental helper code.
    ///
    /// Helper groups and their responsibilities:
    ///   binding helpers         — reference record construction, parameter normalisation, slot
    ///                             read/write, and alias-chain resolution
    ///   alias helpers           — binding-mode transitions for borrow and value assignment
    ///   computed-place helpers  — closures capturing base reference + key for field/index access
    ///   clone helpers           — deep value copy for explicit `copy` semantics
    ///   error helpers           — normalises file paths, constructs canonical error records
    ///   result helpers          — `?` propagation and `or` fallback helpers
    ///   collection helpers      — guarded get/push/remove/length for ordered collections
    ///   map helpers             — guarded get/set/remove and infallible contains/clear/length for ordered maps
    ///   string helpers          — canonical String conversion, equality, and map-key handling
    ///   cast helpers            — numeric and string casting with Result-typed errors
    ///   numeric helpers         — checked i32 and finite f64 arithmetic with trap/Error carriers
    ///   choice helpers          — structural equality for nominal choice carriers
    ///   reactivity helpers      — reactive source bindings, scheduler, and template-string values
    ///
    /// All groups use JS `function` declarations, which are hoisted by the JS engine.
    /// Ordering here is for readability only; correctness does not depend on it.
    pub(crate) fn emit_runtime_prelude(
        &mut self,
        emitted_code_uses_maps: bool,
        emitted_code_uses_numeric_helpers: NumericRuntimeHelperUsage,
        emitted_code_uses_reactive_sources: bool,
        emitted_code_uses_reactive_templates: bool,
    ) {
        self.emit_runtime_binding_helpers();
        self.emit_runtime_alias_helpers();
        self.emit_runtime_computed_place_helpers();
        self.emit_runtime_clone_helpers(emitted_code_uses_maps);
        self.emit_runtime_error_helpers();
        self.emit_runtime_result_helpers();
        self.emit_runtime_collection_helpers();
        if emitted_code_uses_maps {
            self.emit_runtime_map_helpers();
        }
        self.emit_runtime_string_helpers(emitted_code_uses_maps);
        self.emit_runtime_cast_helpers();
        if emitted_code_uses_numeric_helpers.any() {
            self.emit_runtime_numeric_helpers(emitted_code_uses_numeric_helpers);
        }
        if emitted_code_uses_reactive_sources {
            self.emit_runtime_reactive_source_helpers();
        }
        if emitted_code_uses_reactive_templates {
            self.emit_runtime_template_string_helpers();
            self.emit_runtime_mount_helper();
        }
    }
}

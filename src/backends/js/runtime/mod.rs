//! JS runtime helper emission.
//!
//! This module emits JS helper functions that implement Moth's runtime semantics.
//! Most helpers are hoisted `function` declarations; the optional float-power source
//! also initialises constants in the prelude before user code runs.
//!
//! The collection group is the compiler-owned JavaScript implementation of `@core/collections`.
//! [`collection_javascript_helpers`] is consumed by both runtime emission and first-party
//! dependency validation. The remaining prelude groups — binding, alias, computed-place, clone,
//! error, result, map, string, cast, numeric, choice, and reactivity — are compiler-runtime
//! infrastructure rather than first-party package implementations. They intentionally stay out of
//! the first-party package inventory; new package-facing JS must not be added here without an
//! inventory source.
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

pub(crate) use collections::collection_javascript_helpers;

use crate::backends::js::JsEmitter;

/// Describes which checked numeric carrier families are required by emitted JS.
///
/// WHY: each family is shared by every semantic domain with its carrier, while unreachable carrier
///      families stay out of the generated prelude.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NumericRuntimeHelperUsage {
    pub(crate) number_integer_ops: bool,
    pub(crate) big_integer_ops: bool,
    pub(crate) number_decimal_ops: bool,
    pub(crate) binary32_ops: bool,
    pub(crate) binary64_ops: bool,
    pub(crate) binary_float_power: bool,
    pub(crate) format_binary16: bool,
    pub(crate) format_binary32: bool,
    pub(crate) format_binary64: bool,
    pub(crate) validate_float: bool,
}

impl NumericRuntimeHelperUsage {
    pub(crate) fn require_float_formatter(
        &mut self,
        precision: moth_lexical::numeric::precision::BinaryFloatPrecision,
    ) {
        use moth_lexical::numeric::precision::BinaryFloatPrecision;

        match precision {
            BinaryFloatPrecision::Binary16 => self.format_binary16 = true,
            BinaryFloatPrecision::Binary32 => self.format_binary32 = true,
            BinaryFloatPrecision::Binary64 => self.format_binary64 = true,
        }
    }

    pub(crate) fn uses_float_formatter(self) -> bool {
        self.format_binary16 || self.format_binary32 || self.format_binary64
    }

    pub(crate) fn any(self) -> bool {
        self.number_integer_ops
            || self.big_integer_ops
            || self.number_decimal_ops
            || self.binary32_ops
            || self.binary64_ops
            || self.binary_float_power
            || self.uses_float_formatter()
            || self.validate_float
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
    ///   numeric helpers         — checked Number/BigInt integer and profile-precision Float arithmetic
    ///   choice helpers          — structural equality for nominal choice carriers
    ///   reactivity helpers      — reactive source bindings, scheduler, and template-string values
    ///
    /// Most groups use hoisted JS `function` declarations. Float power also initialises
    /// top-level constants, so the complete prelude must precede emitted user functions and start.
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

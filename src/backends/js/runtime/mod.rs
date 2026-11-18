//! JS runtime helper emission.
//!
//! This module emits JS helper functions that implement Moth's runtime semantics.
//! Most helpers are hoisted `function` declarations; the optional float-power source
//! also initialises constants in the prelude before user code runs.
//!
//! The collection group is the compiler-owned JavaScript implementation of `@core/collections`.
//! [`collection_javascript_helpers`] is consumed by both runtime emission and first-party
//! dependency validation. The remaining prelude groups — binding, alias, computed-place, clone,
//! error, map, string, cast, numeric, and choice — are compiler-runtime
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
mod strings;

pub(crate) use collections::collection_javascript_helpers;
pub(crate) use errors::error_result_source;
pub(crate) use numeric::fatal_numeric_trap_throw;

use crate::backends::js::JsEmitter;

/// Describes which checked and fast numeric carriers are required by emitted JS.
///
/// WHY: each checked family is shared by every semantic domain with its carrier, while families
///      no emitted operation can consume stay out of the generated prelude. Demand covers every
///      emitted function because function bodies are emitted even when the start path never calls
///      them. The `Dec` group is split into the shared half-even kernel and the fallible
///      div/idiv/mod/pow family: a positive-scale total multiply carries only the kernel, while
///      total add/subtract/negate and scale-zero multiply carry no numeric code at all. Fast
///      arithmetic tracks each trapping helper separately, so a wrapping operation never carries
///      unused code.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NumericRuntimeHelperUsage {
    pub(crate) number_integer_ops: bool,
    pub(crate) big_integer_ops: bool,
    /// A checked `Dec` division, integer division, remainder or power operation needs the
    /// fallible `__moth_number_*` family.
    pub(crate) number_decimal_ops: bool,
    /// A positive-scale `Dec` multiply, divide or power rounds its coefficient through the shared
    /// half-even kernel. A positive-scale total multiply demands it without the fallible family,
    /// while a scale-zero total multiply is exact coefficient arithmetic and demands neither.
    pub(crate) number_round_half_even: bool,
    pub(crate) binary16_ops: bool,
    pub(crate) binary32_ops: bool,
    pub(crate) binary64_ops: bool,
    /// A checked binary-float `Power` op demands that precision's `pow` member and the shared
    /// power helper.
    pub(crate) binary16_power: bool,
    pub(crate) binary32_power: bool,
    pub(crate) binary64_power: bool,
    pub(crate) format_binary16: bool,
    pub(crate) format_binary32: bool,
    pub(crate) format_binary64: bool,
    pub(crate) validate_float: bool,
    /// A trap-delivered statement consumes the checked `{ tag, value }` helper carrier.
    pub(crate) numeric_trap: bool,
    /// A fast Number-carried integer wraps at its semantic result width.
    pub(crate) fast_number_integer_wrap: bool,
    /// A fast Number-carried integer traps a zero divisor or `MIN // -1`.
    pub(crate) fast_number_integer_division: bool,
    /// A fast Number-carried integer traps a zero divisor.
    pub(crate) fast_number_integer_remainder: bool,
    /// A fast Number-carried integer keeps modular power in the semantic width.
    pub(crate) fast_number_integer_power: bool,
    /// A fast BigInt-carried integer traps a zero divisor or `MIN // -1`.
    pub(crate) fast_big_integer_division: bool,
    /// A fast BigInt-carried integer traps a zero divisor.
    pub(crate) fast_big_integer_remainder: bool,
    /// A fast BigInt-carried integer keeps modular power in the 64-bit width.
    pub(crate) fast_big_integer_power: bool,
    /// Fast binary16 operations need the rounding and finite-trap family.
    pub(crate) fast_binary16_ops: bool,
    /// Fast binary32 operations need the rounding and finite-trap family.
    pub(crate) fast_binary32_ops: bool,
    /// Fast binary64 operations need the finite-trap family.
    pub(crate) fast_binary64_ops: bool,
    /// A fast binary-float `Power` op demands that precision's `pow` member and the shared power
    /// helper.
    pub(crate) fast_binary16_power: bool,
    pub(crate) fast_binary32_power: bool,
    pub(crate) fast_binary64_power: bool,
    /// A fast compound write-back keeps integer low bits and signedness.
    pub(crate) fast_integer_wrap: bool,
    /// A fast compound write-back truncates a binary float into an integer range.
    pub(crate) fast_float_to_integer: bool,
    /// A fast compound write-back rounds a binary float at a destination precision.
    pub(crate) fast_float_conversion: bool,
    /// A fast integer-to-float write-back needs the exact BigInt rounding helper.
    pub(crate) fast_bigint_to_float: bool,
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

    /// Whether any emitted binary-float family member calls the shared power implementation.
    ///
    /// WHY: a family emits its `pow` member only when a selected Power op demands it, so the
    ///      `pow` member and `__moth_float_power` are emitted together. A non-power page carries
    ///      neither and contains no dangling reference.
    pub(crate) fn uses_float_power(self) -> bool {
        self.binary16_power
            || self.binary32_power
            || self.binary64_power
            || self.fast_binary16_power
            || self.fast_binary32_power
            || self.fast_binary64_power
    }

    pub(crate) fn any(self) -> bool {
        self.number_integer_ops
            || self.big_integer_ops
            || self.number_decimal_ops
            || self.number_round_half_even
            || self.binary16_ops
            || self.binary32_ops
            || self.binary64_ops
            || self.uses_float_formatter()
            || self.validate_float
            || self.numeric_trap
            || self.fast_number_integer_wrap
            || self.fast_number_integer_division
            || self.fast_number_integer_remainder
            || self.fast_number_integer_power
            || self.fast_big_integer_division
            || self.fast_big_integer_remainder
            || self.fast_big_integer_power
            || self.fast_binary16_ops
            || self.fast_binary32_ops
            || self.fast_binary64_ops
            || self.fast_integer_wrap
            || self.fast_float_to_integer
            || self.fast_float_conversion
            || self.fast_bigint_to_float
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
    ///   collection helpers      — guarded get/push/remove/length for ordered collections
    ///   map helpers             — guarded get/set/remove and infallible contains/clear/length for ordered maps
    ///   string helpers          — generic value-to-string conversion
    ///   cast helpers            — numeric and string casting with Result-typed errors
    ///   numeric helpers         — checked Number/BigInt integer and exact-precision binary-float
    ///                             arithmetic plus the shared Dec half-even kernel and its fallible
    ///                             family; a positive-scale total Dec multiply still consumes the
    ///                             kernel, while other total scalar operations emit no helper
    ///   choice helpers          — structural equality for nominal choice carriers
    ///
    /// Most groups use hoisted JS `function` declarations. Binary-float power also initialises
    /// top-level constants, so the complete prelude must precede emitted user functions and start.
    pub(crate) fn emit_runtime_prelude(
        &mut self,
        emitted_code_uses_maps: bool,
        emitted_code_uses_numeric_helpers: NumericRuntimeHelperUsage,
    ) {
        self.emit_runtime_binding_helpers();
        self.emit_runtime_alias_helpers();
        self.emit_runtime_computed_place_helpers();
        self.emit_runtime_clone_helpers(emitted_code_uses_maps);
        self.emit_runtime_error_helpers();
        self.emit_runtime_collection_helpers();
        if emitted_code_uses_maps {
            self.emit_runtime_map_helpers();
        }
        self.emit_runtime_string_helpers(emitted_code_uses_maps);
        let mut emitted_helpers = self.emit_runtime_cast_helpers();
        if emitted_code_uses_numeric_helpers.any() {
            self.emit_runtime_numeric_helpers(
                emitted_code_uses_numeric_helpers,
                &mut emitted_helpers,
            );
        }
    }
}

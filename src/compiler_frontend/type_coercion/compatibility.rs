//! Type compatibility checks for the Moth compiler frontend.
//!
//! WHAT: determines whether a value of a given type is accepted in a position
//! expecting a target type.
//! WHY: this is the sole owner of compatibility policy. All call sites that
//! need to check type compatibility must go through `is_type_compatible` so
//! that structural compatibility rules are applied consistently.
//! `datatypes.rs` owns type structure only; it no longer carries any
//! compatibility logic.

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::queries::TypeKind;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use rustc_hash::FxHashMap;

/// Semantic compatibility mode for cache entries.
///
/// WHAT: call validation has one narrow extension over ordinary compatibility:
/// fresh collection rvalues may satisfy mutable collection slots after HIR
/// materializes them into hidden locals.
/// WHY: the cache key must include that boundary-specific policy so a standard
/// mismatch is not accidentally reused as a mutable-rvalue success or vice versa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TypeCompatibilityMode {
    Standard,
    FreshMutableRvalue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TypeCompatibilityKey {
    expected_id: TypeId,
    actual_id: TypeId,
    mode: TypeCompatibilityMode,
}

/// Module-local memoization for pure type-compatibility results.
///
/// The cache stores only semantic booleans. Source locations and diagnostic
/// context stay with the caller so errors remain precise and cannot leak across
/// different call sites.
#[derive(Default)]
pub(crate) struct TypeCompatibilityCache {
    entries: FxHashMap<TypeCompatibilityKey, bool>,
}

impl TypeCompatibilityCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_compatible(
        &mut self,
        expected_id: TypeId,
        actual_id: TypeId,
        mode: TypeCompatibilityMode,
        type_environment: &TypeEnvironment,
    ) -> bool {
        // Exact canonical identity is the common compatibility case. Return before
        // hashing so the cache stays focused on structural and boundary-specific checks.
        if expected_id == actual_id {
            return true;
        }

        increment_frontend_counter(FrontendCounter::TypeCompatibilityCacheLookups);

        let key = TypeCompatibilityKey {
            expected_id,
            actual_id,
            mode,
        };

        if let Some(result) = self.entries.get(&key) {
            increment_frontend_counter(FrontendCounter::TypeCompatibilityCacheHits);
            return *result;
        }

        let result = match mode {
            TypeCompatibilityMode::Standard => {
                is_type_compatible(expected_id, actual_id, type_environment)
            }
            TypeCompatibilityMode::FreshMutableRvalue => {
                fresh_mutable_rvalue_type_compatible(expected_id, actual_id, type_environment)
            }
        };

        increment_frontend_counter(FrontendCounter::TypeCompatibilityCacheMisses);
        self.entries.insert(key, result);

        result
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Returns true when `actual_id` is acceptable in a position that expects `expected_id`.
///
/// WHAT: the central compatibility predicate for all type positions.
/// WHY: centralising here keeps structural compatibility rules out of parser
/// and lowering sites.
///
/// Rules:
/// - `Option<T>` accepts `None`, `T`, and `Option<T>`.
/// - `StringSlice` accepts `Template` and `TemplateWrapper` (all lower to the same HIR type).
///   This is handled naturally because both map to the same `String` TypeId.
/// - All other cases require `TypeId` equality.
pub(crate) fn is_type_compatible(
    expected_id: TypeId,
    actual_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    if expected_id == actual_id {
        return true;
    }

    // Struct compatibility includes const-record and generic-instance rules.
    if is_struct_type(expected_id, type_environment) && is_struct_type(actual_id, type_environment)
    {
        return struct_types_compatible(expected_id, actual_id, type_environment);
    }

    // Option compatibility: Option<T> accepts T, None, and Option<T>.
    if type_environment.is_option(expected_id) {
        if actual_id == type_environment.builtins().none {
            return true;
        }

        let Some(expected_inner) = type_environment.option_inner_type(expected_id) else {
            return false;
        };

        if expected_inner == actual_id {
            return true;
        }

        if type_environment.is_option(actual_id) {
            let Some(actual_inner) = type_environment.option_inner_type(actual_id) else {
                return false;
            };

            if actual_inner == type_environment.builtins().none
                || expected_inner == type_environment.builtins().none
            {
                return true;
            }

            return actual_inner == expected_inner;
        }
    }

    // Template and TemplateWrapper both map to the String builtin TypeId,
    // so they are naturally compatible with String via TypeId equality.
    // Everything else requires exact TypeId equality.
    expected_id == actual_id
}

/// Returns true when postfix propagation can bubble `actual_error_id` into a function whose
/// fallible error slot is `expected_error_id`.
///
/// WHAT: postfix `!` has a narrower policy than ordinary type compatibility: exact equality or
/// one-level contextual option wrapping from `E` into `E?`.
/// WHY: fallible propagation must not normalize raw result values through the general
/// compatibility lattice, but it still needs the approved optional-error boundary.
pub(crate) fn is_postfix_error_compatible(
    expected_error_id: TypeId,
    actual_error_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    expected_error_id == actual_error_id
        || type_environment.option_inner_type(expected_error_id) == Some(actual_error_id)
}

/// Returns true when `actual_id` is acceptable at an explicit declaration site
/// expecting `expected_id`.
///
/// WHAT: the compatibility predicate for `result T = expr` declarations.
/// WHY: declarations accept exact structural matches plus the single implicit
/// numeric promotion `Int → Float`.
pub(crate) fn is_declaration_compatible(
    expected_id: TypeId,
    actual_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    is_type_compatible(expected_id, actual_id, type_environment)
        || is_numeric_coercible_by_id(actual_id, expected_id, type_environment)
}

/// Returns true when `actual_id` can be implicitly promoted to `expected_id` as a
/// contextual numeric coercion.
///
/// WHAT: the narrow set of implicit numeric promotions the language allows.
/// WHY: only Int → Float is supported today. All other numeric combinations
/// require explicit user casts (`Float(x)` / `Int(x)`).
pub(crate) fn is_numeric_coercible_by_id(
    actual_id: TypeId,
    expected_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    actual_id == type_environment.builtins().int && expected_id == type_environment.builtins().float
}

// --------------------------------------------------------
// Internal helpers
// --------------------------------------------------------

fn is_struct_type(id: TypeId, type_environment: &TypeEnvironment) -> bool {
    matches!(
        type_environment.type_kind(id),
        Some(TypeKind::Struct | TypeKind::GenericInstance)
    )
}

fn struct_types_compatible(
    expected: TypeId,
    actual: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    if type_environment.is_const_record(expected) != type_environment.is_const_record(actual) {
        return false;
    }

    // Generic instances are interned, so equality is the right check.
    if type_environment.generic_instance_key(expected).is_some()
        || type_environment.generic_instance_key(actual).is_some()
    {
        return expected == actual;
    }

    // For base structs, compare by nominal path.
    type_environment.nominal_path(expected) == type_environment.nominal_path(actual)
}

fn fresh_mutable_rvalue_type_compatible(
    expected_id: TypeId,
    actual_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    if is_type_compatible(expected_id, actual_id, type_environment) {
        return true;
    }

    // Fresh collection literals are produced as immutable-owned values by default, but
    // mutable call slots own and materialize their own hidden local before the call.
    // Inner element type compatibility still has to hold.
    if let (Some(expected_shape), Some(actual_shape)) = (
        type_environment.collection_shape(expected_id),
        type_environment.collection_shape(actual_id),
    ) {
        return expected_shape.fixed_capacity
            == type_environment.collection_fixed_capacity(actual_id)
            && is_type_compatible(
                expected_shape.element_type,
                actual_shape.element_type,
                type_environment,
            );
    }

    false
}

#[cfg(test)]
#[path = "tests/compatibility_tests.rs"]
mod compatibility_tests;

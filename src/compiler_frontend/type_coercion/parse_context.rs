//! Parse-time context helpers for the Moth compiler frontend.
//!
//! WHAT: determines what expected type should be threaded into expression
//! parsing at assignment-like sites (declarations, mutations, struct fields,
//! collection items).
//! WHY: most types should be resolved strictly by the expression parser with
//! no expected-type hint (`Inferred`), so that `eval_expression` operates in
//! `Exact` context and callers own their own post-parse coercion. However, some
//! literals are parse-context-sensitive: `none` needs an `Option(_)` target, and
//! empty collection literals need an explicit `Collection(T)` target. Without
//! those hints the parser must reject the literal immediately, because there is
//! no post-parse coercion that can "invent" a missing inner type.
//!
//! ## Rule
//!
//! - `Option(_)` targets: pass the full option type through so `none` can
//!   extract its inner type at parse time.
//! - Explicit `Collection(T)` targets: pass the collection type through so
//!   empty collection literals can resolve their element type.
//! - All other targets: pass `Inferred` so the expression resolves its own
//!   natural type and the call site validates/coerces after the fact.

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastTarget, cast_target_for_receiving_type,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Parse-time expected type for context-sensitive literals such as `none` and
/// empty collection literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpectedType {
    Known(TypeId),
    Infer,
}

impl ExpectedType {
    pub(crate) fn known_type_id(self) -> Option<TypeId> {
        match self {
            Self::Known(type_id) => Some(type_id),
            Self::Infer => None,
        }
    }
}

/// Explicit cast target supplied by a receiving boundary.
///
/// WHAT: carries the builtin target type from a typed boundary into expression
///      parsing so that a leading `cast` / `cast!` keyword can resolve its
///      target without making ordinary expression parsing globally type-directed.
/// WHY: `ExpectedType` owns parse-time literal context only; cast target
///      ownership is intentionally separate so boundary callers can offer an
///      explicit cast channel while keeping `parse_expectation_for_type_id`
///      unchanged for normal expressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CastTargetContext {
    /// No explicit cast target is available; `cast` is invalid at this boundary.
    None,

    /// The boundary is explicit, but the target is a generic parameter.
    TargetIsGenericParameter { target_type_id: TypeId },

    /// The boundary is explicit, but the target is not one of the builtin cast targets.
    TargetNotBuiltin { target_type_id: TypeId },

    /// The receiving boundary has an explicit builtin target type.
    ExplicitBoundary {
        target_type_id: TypeId,
        target: BuiltinCastTarget,
        requires_optional_wrap_after_cast: bool,
    },
}

/// Returns the expected type that should be passed into expression parsing
/// for a given target type.
///
/// WHAT: the single authoritative rule for what parse-time context is
/// preserved at assignment-like sites.
/// WHY: only parse-context-sensitive literals keep target context. All other
/// targets use `Inferred` to keep the expression parser strict and push
/// coercion ownership to the call site.
pub(crate) fn parse_expectation_for_type_id(
    target_id: TypeId,
    type_environment: &TypeEnvironment,
) -> ExpectedType {
    if type_environment.is_option(target_id) {
        return ExpectedType::Known(target_id);
    }

    if type_environment.is_collection(target_id) {
        let collection_shape = type_environment.collection_shape(target_id);
        if collection_shape.map(|shape| shape.element_type)
            != Some(type_environment.builtins().none)
        {
            return ExpectedType::Known(target_id);
        }
    }

    if type_environment.is_map_type(target_id) {
        return ExpectedType::Known(target_id);
    }

    ExpectedType::Infer
}

/// Builds a `CastTargetContext` from a receiving type.
///
/// WHAT: classifies the receiving type as a builtin cast target and records
///      whether the cast should land in the inner type before optional wrapping.
/// WHY: boundary callers should not re-derive the optional-unwrapping rule
///      when building the cast target channel.
pub(crate) fn cast_target_context_for_type_id(
    target_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> CastTargetContext {
    let resolution = match cast_target_for_receiving_type(target_id, type_environment, string_table)
    {
        Some(resolution) => resolution,
        None => {
            let diagnostic_target_id = type_environment
                .option_inner_type(target_id)
                .unwrap_or(target_id);

            if matches!(
                type_environment.get(diagnostic_target_id),
                Some(TypeDefinition::GenericParameter(_))
            ) {
                return CastTargetContext::TargetIsGenericParameter {
                    target_type_id: diagnostic_target_id,
                };
            }

            return CastTargetContext::TargetNotBuiltin {
                target_type_id: diagnostic_target_id,
            };
        }
    };

    let target_type_id = if resolution.requires_optional_wrap_after_cast {
        type_environment
            .option_inner_type(target_id)
            .unwrap_or(target_id)
    } else {
        target_id
    };

    CastTargetContext::ExplicitBoundary {
        target_type_id,
        target: resolution.target,
        requires_optional_wrap_after_cast: resolution.requires_optional_wrap_after_cast,
    }
}

/// Collection-specific parse-time context passed into collection literal parsing.
///
/// WHAT: replaces `Option<TypeId>` element hints with the full collection shape
///       so fixed capacity, element type, and exact semantic identity are available
///       when the literal is parsed against an explicit or inferred target.
/// WHY: collection literal behavior differs between growable, fixed, and shorthand
///      declaration contexts; the parser needs the full shape to validate length
///      and produce the correct canonical `TypeId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpectedCollectionContext {
    /// Explicit collection annotation (growable or fixed) with known element type.
    Explicit {
        collection_type_id: TypeId,
        element_type_id: TypeId,
        fixed_capacity: Option<usize>,
    },

    /// Capacity-only shorthand `{N}`: element must be inferred from the literal.
    CapacityOnlyShorthand { fixed_capacity: usize },
}

/// Map-specific parse-time context passed into map literal parsing.
///
/// WHAT: carries the full map shape so the parser can validate keys, coerce
///       values, and produce the correct canonical `TypeId`.
/// WHY: map literal behavior differs from collections; the parser needs both
///      key and value types to validate entries and detect empty-literal errors.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExpectedMapContext {
    pub(crate) key_type_id: TypeId,
    pub(crate) value_type_id: TypeId,
    pub(crate) key_diagnostic_type: DataType,
    pub(crate) value_diagnostic_type: DataType,
    pub(crate) map_type_id: Option<TypeId>,
}

/// Unified curly-brace literal context that distinguishes collections, maps,
/// and inferred targets so the parser can dispatch to the correct literal shape.
///
/// WHAT: replaces the raw `ExpectedCollectionContext` at the expression-dispatch
///       boundary so `{...}` can lower to either a collection or a map.
/// WHY: the same source token (`{`) introduces two different literal shapes;
///      the decision is made from the expected type or from the first entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExpectedCurlyLiteralContext {
    /// No annotation: the literal shape must be discovered from the first entry.
    Infer,
    /// Explicit or contextual collection type.
    Collection(ExpectedCollectionContext),
    /// Explicit or contextual map type.
    Map(ExpectedMapContext),
}

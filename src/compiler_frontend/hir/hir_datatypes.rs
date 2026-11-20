//! HIR Type Classification.
//!
//! HIR carries frontend semantic `TypeId`s directly.
//! There is no separate HIR type interner; `TypeEnvironment` owns canonical identity.
//!
//! This module provides backend-agnostic type classification helpers that
//! backends use to decide ABI, lowering strategy, and runtime representation.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;

use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, TypeId};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Backend-agnostic classification of a HIR type.
///
/// WHAT: collapses the full frontend type taxonomy into the coarse categories backends care about
/// (scalar vs heap vs void vs function) so each backend only needs a small match table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirTypeClass {
    Unit,
    Bool,
    Char,
    /// Explicit-width builtin scalar or the `Byte` octet type.
    ///
    /// WHAT: keeps the exact-width identity visible to backends instead of erasing it into a
    ///       coarse numeric class.
    /// WHY: ABI and layout decisions must be able to distinguish `U8` from `I32`, and `Byte`
    ///      from `U8`, so classification carries the scalar rather than a coarse numeric class.
    FixedScalar(FixedScalar),
    /// One arbitrary-precision `Dec` scale identity.
    ///
    /// WHAT: carries the semantic scale so backends can distinguish the `Dec` family from
    ///       every fixed scalar and let each target's feature validation gate reachable Dec values.
    /// WHY: collapsing the internal `Number` variant into `HeapAllocated` or a scalar
    ///      substitute would erase the identity that target gates and transport decisions
    ///      need.
    Number(NumberScale),
    Function,
    HeapAllocated,
}

impl HirTypeClass {
    /// Classifies one registered type definition.
    ///
    /// WHAT: returns `None` for the compile-time-only anonymous const-record marker, which has no
    ///       executable HIR representation.
    /// WHY: the marker needs its own error at the `classify_hir_type` boundary, while the
    ///      executable class rule stays in one place for every caller.
    fn from_definition(definition: &TypeDefinition) -> Option<Self> {
        match definition {
            TypeDefinition::Builtin(builtin) => Some(match builtin.key {
                BuiltinTypeKey::Bool => Self::Bool,
                BuiltinTypeKey::Number(scale) => Self::Number(scale),
                BuiltinTypeKey::Char => Self::Char,
                BuiltinTypeKey::None => Self::Unit,
                BuiltinTypeKey::FixedScalar(scalar) => Self::FixedScalar(scalar),
                BuiltinTypeKey::String | BuiltinTypeKey::Range => Self::HeapAllocated,
            }),

            TypeDefinition::Struct(..)
            | TypeDefinition::Choice(..)
            | TypeDefinition::Constructed(..)
            | TypeDefinition::External(..)
            | TypeDefinition::GenericInstance(..)
            | TypeDefinition::GenericParameter(..) => Some(Self::HeapAllocated),

            TypeDefinition::Function(..) => Some(Self::Function),

            TypeDefinition::AnonymousConstRecordMarker => None,
        }
    }

    /// Whether mutating a place of this type can change storage another binding observes.
    ///
    /// WHAT: false for value classes whose mutable access replaces the place's own value.
    ///       Bool, Char, None, fixed scalars and `Dec` scales have no in-place mutation surface.
    /// WHY: a definite slot storing a produced value keeps its own binding cell even when the
    ///      value's provenance aliases an older allocation. Writing that cell cannot reach the
    ///      older allocation for a value class, while heap-allocated types (aggregates, maps,
    ///      collections, strings and opaque resources) store shared references or expose mutable
    ///      receivers, so a callee could mutate the aliased storage in place.
    pub(crate) fn has_in_place_mutation_surface(self) -> bool {
        matches!(self, Self::HeapAllocated | Self::Function)
    }
}

/// Classifies a frontend `TypeId` into a backend-agnostic category.
///
/// WHAT: queries `TypeEnvironment` for the definition behind `type_id` and maps it to the
///       coarse class that backends use for ABI/layout decisions.
/// WHY: backends should not pattern-match on frontend `TypeDefinition` variants directly;
///      this function keeps the classification logic in one place.
pub fn classify_hir_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<HirTypeClass, InfrastructureFailure> {
    let Some(definition) = type_environment.get(type_id) else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HirConstruction,
            format!("Unregistered TypeId {type_id:?} reached HIR type classification"),
        ));
    };

    HirTypeClass::from_definition(definition).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::HirConstruction,
            "compile-time-only anonymous const-record marker reached executable HIR type classification",
        )
    })
}

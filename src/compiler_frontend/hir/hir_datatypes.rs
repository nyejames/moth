//! HIR Type Classification.
//!
//! HIR carries frontend semantic `TypeId`s directly.
//! There is no separate HIR type interner; `TypeEnvironment` owns canonical identity.
//!
//! This module provides backend-agnostic type classification helpers that
//! backends use to decide ABI, lowering strategy, and runtime representation.

use crate::compiler_frontend::compiler_errors::CompilerError;

use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, TypeId};

/// Backend-agnostic classification of a HIR type.
///
/// WHAT: collapses the full frontend type taxonomy into the coarse categories backends care about
/// (scalar vs heap vs void vs function) so each backend only needs a small match table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirTypeClass {
    Unit,
    Bool,
    Char,
    Int,
    Float,
    Function,
    HeapAllocated,
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
) -> Result<HirTypeClass, CompilerError> {
    let Some(definition) = type_environment.get(type_id) else {
        return Err(CompilerError::compiler_error(format!(
            "Unregistered TypeId {type_id:?} reached HIR type classification"
        )));
    };

    match definition {
        TypeDefinition::Builtin(builtin) => Ok(match builtin.key {
            BuiltinTypeKey::Bool => HirTypeClass::Bool,
            BuiltinTypeKey::Int => HirTypeClass::Int,
            BuiltinTypeKey::Float => HirTypeClass::Float,
            // Decimal is intentionally inactive in the Alpha surface. Classify it as
            // heap-allocated so no backend lowers it as a numeric scalar.
            BuiltinTypeKey::Decimal => HirTypeClass::HeapAllocated,
            BuiltinTypeKey::Char => HirTypeClass::Char,
            BuiltinTypeKey::None => HirTypeClass::Unit,
            BuiltinTypeKey::String | BuiltinTypeKey::Range => HirTypeClass::HeapAllocated,
        }),

        TypeDefinition::Struct(..)
        | TypeDefinition::Choice(..)
        | TypeDefinition::Constructed(..)
        | TypeDefinition::External(..)
        | TypeDefinition::GenericInstance(..)
        | TypeDefinition::GenericParameter(..) => Ok(HirTypeClass::HeapAllocated),

        TypeDefinition::Function(..) => Ok(HirTypeClass::Function),

        TypeDefinition::AnonymousConstRecordMarker => Err(CompilerError::compiler_error(
            "compile-time-only anonymous const-record marker reached executable HIR type classification",
        )),
    }
}

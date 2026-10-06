//! Shared frontend type-annotation syntax helpers.
//!
//! WHAT: owns parsing of explicit type annotations into `ParsedTypeRef` and diagnostic
//! `DataType` spelling, plus parsed-ref traversal for dependency discovery.
//! WHY: header parsing and body-local AST parsing both need the same token-to-type syntax,
//!      but semantic resolution into `TypeId` is AST-owned.
//!
//! This module owns:
//! - token-to-type annotation parsing for declaration/signature contexts
//! - the single builtin-scalar spelling owner: token tag to `FixedScalar` identity and back
//! - optional suffix (`?`) annotation rules
//! - collection type parsing with fixed-capacity syntax (e.g. `{64 Int}`, `{T}`)
//! - `parsed_ref_to_data_type` syntax-to-diagnostic spelling
//! - parsed-ref walkers used by header dependency extraction
//!
//! This module does NOT own:
//! - semantic type resolution into canonical `TypeId` (lives in `ast::type_resolution`)
//! - declaration/statement-level semantics (mutability rules, initializer rules)
//! - expression typing/coercion policy
//! - call-site/feature-specific diagnostic framing outside type syntax itself

use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::reserved_trait_keyword_or_dispatch_mismatch_for_tag;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, GenericApplicationErrorReason, InvalidCollectionTypeReason,
    InvalidMapTypeReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::generic_identity_bridge::GenericBaseType;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

pub(crate) use crate::compiler_frontend::compiler_messages::TypeAnnotationContext;

mod parse;
mod walk;

pub(crate) use parse::*;
pub(crate) use walk::*;

/// Maps a builtin scalar keyword tag to its explicit-width identity.
///
/// WHAT: the one owner of `TokenTag -> FixedScalar`. It covers only the explicit-width
///       spellings (`I8`..`F64`) and `Byte`, which are profile-independent identities.
/// WHY: parsing, expression diagnostics and header classification all need this mapping, and a
///      second copy would let the keyword set and the identity set drift apart. `Int`, `Uint`
///      and `Float` deliberately return `None`: they are profile-dependent identities with no
///      fixed width (`Uint` follows the profile-selected `Int` width).
pub(crate) fn fixed_scalar_for_builtin_type_tag(tag: TokenTag) -> Option<FixedScalar> {
    match tag {
        TokenTag::DATATYPE_I8 => Some(FixedScalar::I8),
        TokenTag::DATATYPE_I16 => Some(FixedScalar::I16),
        TokenTag::DATATYPE_I32 => Some(FixedScalar::I32),
        TokenTag::DATATYPE_I64 => Some(FixedScalar::I64),
        TokenTag::DATATYPE_U8 => Some(FixedScalar::U8),
        TokenTag::DATATYPE_U16 => Some(FixedScalar::U16),
        TokenTag::DATATYPE_U32 => Some(FixedScalar::U32),
        TokenTag::DATATYPE_U64 => Some(FixedScalar::U64),
        TokenTag::DATATYPE_F16 => Some(FixedScalar::F16),
        TokenTag::DATATYPE_F32 => Some(FixedScalar::F32),
        TokenTag::DATATYPE_F64 => Some(FixedScalar::F64),
        TokenTag::DATATYPE_BYTE => Some(FixedScalar::Byte),
        _ => None,
    }
}

/// Returns the source spelling of a builtin scalar type keyword tag.
///
/// WHAT: covers `Int`, `Uint`, `Float`, `Bool`, `String`, `Char` and every explicit-width
///       spelling.
/// WHY: diagnostics and receiver surfaces must spell builtin scalar types exactly as authored, so
///      the spelling comes from `FixedScalar::name` for fixed scalars instead of a second table.
pub(crate) fn builtin_scalar_type_name_for_tag(tag: TokenTag) -> Option<&'static str> {
    match tag {
        TokenTag::DATATYPE_INT => Some("Int"),
        TokenTag::DATATYPE_UINT => Some("Uint"),
        TokenTag::DATATYPE_FLOAT => Some("Float"),
        TokenTag::DATATYPE_BOOL => Some("Bool"),
        TokenTag::DATATYPE_STRING => Some("String"),
        TokenTag::DATATYPE_CHAR => Some("Char"),
        _ => fixed_scalar_for_builtin_type_tag(tag).map(FixedScalar::name),
    }
}

/// Convert parsed type syntax to a diagnostic `DataType` spelling.
///
/// WHAT: produces a `DataType` for parse-only and diagnostic-only contexts.
/// WHY: parsed type annotations start as `ParsedTypeRef` and are resolved to `TypeId`
///      for semantic identity; this function is only for display/compatibility paths.
pub(crate) fn parsed_ref_to_data_type(parsed: &ParsedTypeRef) -> DataType {
    match parsed {
        ParsedTypeRef::Inferred => DataType::Inferred,
        ParsedTypeRef::BuiltinBool { .. } => DataType::Bool,
        ParsedTypeRef::BuiltinInt { .. } => DataType::Int,
        ParsedTypeRef::BuiltinUint { .. } => DataType::Uint,
        ParsedTypeRef::BuiltinFloat { .. } => DataType::Float,
        ParsedTypeRef::BuiltinString { .. } => DataType::StringSlice,
        ParsedTypeRef::BuiltinChar { .. } => DataType::Char,
        ParsedTypeRef::BuiltinFixedScalar { scalar, .. } => DataType::FixedScalar(*scalar),
        ParsedTypeRef::BuiltinNumber { scale, .. } => DataType::Number(*scale),
        ParsedTypeRef::Named { name, .. } => DataType::NamedType(*name),
        ParsedTypeRef::Qualified { path, .. } => DataType::NamespacedType { path: path.clone() },
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            let base_dt = parsed_ref_to_data_type(base);
            let base = match base_dt {
                DataType::NamedType(type_name) => GenericBaseType::Named(type_name),
                _ => {
                    // Fallback for unsupported base shapes in diagnostic-only paths.
                    return DataType::Inferred;
                }
            };
            DataType::GenericInstance {
                base,
                arguments: arguments.iter().map(parsed_ref_to_data_type).collect(),
            }
        }
        ParsedTypeRef::Collection { element, .. } => {
            DataType::collection(parsed_ref_to_data_type(element))
        }
        ParsedTypeRef::Map { key, value, .. } => {
            DataType::map(parsed_ref_to_data_type(key), parsed_ref_to_data_type(value))
        }
        ParsedTypeRef::Optional { inner, .. } => {
            DataType::Option(Box::new(parsed_ref_to_data_type(inner)))
        }

        ParsedTypeRef::This { .. } => DataType::Inferred,
    }
}

#[cfg(test)]
#[path = "../tests/type_syntax_tests.rs"]
mod type_syntax_tests;

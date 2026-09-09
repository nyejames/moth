//! Shared frontend type-annotation syntax helpers.
//!
//! WHAT: owns parsing of explicit type annotations into `ParsedTypeRef` and diagnostic
//! `DataType` spelling, plus parsed-ref traversal for dependency discovery.
//! WHY: header parsing and body-local AST parsing both need the same token-to-type syntax,
//!      but semantic resolution into `TypeId` is AST-owned.
//!
//! This module owns:
//! - token-to-type annotation parsing for declaration/signature contexts
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

use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::reserved_trait_keyword_or_dispatch_mismatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, GenericApplicationErrorReason, InvalidCollectionTypeReason,
    InvalidMapTypeReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::generic_identity_bridge::GenericBaseType;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

pub(crate) use crate::compiler_frontend::compiler_messages::TypeAnnotationContext;

mod parse;
mod walk;

pub(crate) use parse::*;
pub(crate) use walk::*;

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
        ParsedTypeRef::BuiltinFloat { .. } => DataType::Float,
        ParsedTypeRef::BuiltinString { .. } => DataType::StringSlice,
        ParsedTypeRef::BuiltinChar { .. } => DataType::Char,
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

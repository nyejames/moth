//! Source-authored map type policy for type resolution.
//!
//! WHAT: owns the two policy checks that apply to map types written directly in
//!       source code: inline nesting readability validation and V1 scalar-key validation.
//! WHY: these rules are independent from alias expansion, generic instantiation,
//!      and `TypeEnvironment` shape interning; keeping them in a focused module lets
//!      `resolve_type.rs` concentrate on orchestrating those larger concerns while
//!      this module owns the narrow map-specific policy.
//!
//! This module owns:
//! - counting inline map nesting depth in parsed type references and rejecting depth
//!   greater than two before the map type is resolved.
//! - accepting only `String`, `Int`, `Bool`, and `Char` as supported V1 map keys.
//!
//! This module does NOT own:
//! - `TypeEnvironment::intern_map` or canonical map `TypeId` construction.
//! - generic nominal instantiation or bound-evidence checks for map arguments.
//! - map literal parsing or map member validation in AST bodies.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidMapTypeReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Computes the maximum inline map nesting depth for a parsed type reference.
///
/// WHAT: counts how many levels of map types are nested within the given parsed ref.
/// WHY: readability validation limits inline map nesting to two levels; named aliases
///      reset the depth because they provide a named abstraction.
///
/// Depth rules:
/// - `{String = Int}` has depth 1.
/// - `{String = {String = Int}}` has depth 2.
/// - `{String = {String = {String = Int}}}` has depth 3 and is rejected.
/// - Named types (`MyAlias`) and namespaced types (`Ns::Alias`) reset depth to 0
///   because the name abstracts the shape.
pub(super) fn map_nesting_depth(parsed: &ParsedTypeRef) -> usize {
    match parsed {
        ParsedTypeRef::Map { key, value, .. } => {
            let key_depth = map_nesting_depth(key);
            let value_depth = map_nesting_depth(value);
            1 + key_depth.max(value_depth)
        }
        ParsedTypeRef::Named { .. } | ParsedTypeRef::Qualified { .. } => 0,
        ParsedTypeRef::Collection { element, .. } => map_nesting_depth(element),
        ParsedTypeRef::Optional { inner, .. } => map_nesting_depth(inner),
        ParsedTypeRef::Applied { arguments, .. } => {
            arguments.iter().map(map_nesting_depth).max().unwrap_or(0)
        }
        _ => 0,
    }
}

/// Validates that a map key type is supported for V1 ordered maps.
///
/// WHAT: accepts only `String`, `Int`, `Bool`, and `Char` keys.
/// WHY: builtin maps are deliberately scalar-keyed. This helper is the canonical owner
///      of that policy, so generic parameters and user-defined types follow the same
///      rejection path as every other unsupported key.
pub(crate) fn validate_map_key_type(
    key_type_id: TypeId,
    type_environment: &TypeEnvironment,
    location: &SourceLocation,
) -> Result<(), Box<CompilerDiagnostic>> {
    let builtins = type_environment.builtins();
    let is_supported_scalar = key_type_id == builtins.string
        || key_type_id == builtins.int
        || key_type_id == builtins.bool
        || key_type_id == builtins.char;

    if is_supported_scalar {
        return Ok(());
    }

    Err(Box::new(CompilerDiagnostic::invalid_map_type(
        InvalidMapTypeReason::UnsupportedKeyType {
            key_type: key_type_id,
        },
        location.clone(),
    )))
}

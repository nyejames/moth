//! Source-visible type-name lookup and trait-name rejection for AST type resolution.
//!
//! WHAT: resolves source-authored bare and namespace-qualified type names to the declarations,
//!       aliases, external types, generic parameters, builtins, and traits visible in the current
//!       type-resolution context. It also rejects names that are values, traits, or generic bases
//!       used in positions where a concrete type is required.
//! WHY: name lookup is a self-contained concern that touches visibility records, declaration
//!      tables, namespace records, generic scopes, and trait environments; keeping it in a focused
//!      module lets `resolve_type.rs` concentrate on the overall parsed-ref orchestration and
//!      `TypeId` conversion.
//!
//! This module owns:
//! - resolving bare named types (`DataType::NamedType`) to visible declarations, aliases, external
//!   types, generic parameters, builtins, or trait-name rejections.
//! - resolving namespace-qualified types (`DataType::NamespacedType`) to visible namespace type
//!   members or namespace type/value misuse diagnostics.
//! - resolving generic application bases (`GenericBaseType::Named`) to declared generic structs/
//!   choices and rejecting other generic-base misuses.
//! - rejecting bare generic type names used without type arguments.
//! - rejecting trait names used as ordinary types.
//!
//! This module does NOT own:
//! - `TypeResolutionContext` construction or result types (live in `context.rs`).
//! - type alias target re-resolution (lives in `aliases.rs`).
//! - lazy generic struct/choice instance materialization or bound-evidence validation
//!   (lives in `generics.rs`).
//! - diagnostic-type-to-`TypeId` conversion helpers (live in `resolve_type.rs`).
//! - collection capacity folding or map key validation (live in `collections.rs` and `maps.rs`).

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult, aliases, context::TypeResolutionContext,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason, InvalidTypeAnnotationReason,
    NameNamespace, NamespaceTypeValueMisuseKind,
};
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    BuiltinGenericType, GenericBaseType,
};
use crate::compiler_frontend::datatypes::{DataType, diagnostic_type_spelling};
use crate::compiler_frontend::declaration_syntax::type_syntax::TypeAnnotationContext;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::{
    NamespaceMemberLookup, NamespaceRecordSource, NamespaceTypeMember, lookup_namespace_member,
};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{SourceLocation, TokenKind};
use rustc_hash::FxHashSet;

/// Resolve a bare named type from the current type-resolution context.
///
/// WHAT: searches generic parameters, visible type aliases, visible source declarations,
///       visible external types, visible traits, and builtin type names in order, returning the
///       resolved diagnostic spelling or an appropriate diagnostic when the name cannot be used
///       as a type.
/// WHY: this is the single lookup path for `DataType::NamedType`; centralizing it keeps the
///      priority order between parameters, aliases, declarations, and builtins explicit.
pub(super) fn resolve_named_type_from_context(
    type_name: StringId,
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> TypeResolutionResult<DataType> {
    increment_ast_counter(AstCounter::VisibleTypeLookupAttempts);

    // 1) Generic parameter scope.
    if let Some(generic_scope) = context.generic_parameters
        && let Some(parameter) = generic_scope.resolve(type_name)
    {
        if let Some(canonical_id) = parameter.canonical_id
            && let Some(substitutions) = context.generic_substitutions
            && let Some(concrete_type_id) = substitutions.get(&canonical_id).copied()
        {
            return Ok(diagnostic_type_spelling(
                concrete_type_id,
                context.type_environment,
            ));
        }

        return Ok(DataType::TypeParameter {
            id: parameter.local_id,
            canonical_id: parameter.canonical_id,
            name: parameter.name,
        });
    }

    // 2) Visible type aliases.
    //
    // Reuse the alias module's lookup helper so the same visibility rules apply here and in
    // parsed-ref alias expansion. Aliases are resolved before they are stored, so the cached
    // diagnostic spelling is already the expanded target.
    if let Some((_alias_path, annotation)) =
        aliases::visible_type_alias_annotation(type_name, context)
    {
        return Ok(annotation.diagnostic_type.clone());
    }

    // 3) Visible source declarations (path-based first, then name fallback).
    increment_ast_counter(AstCounter::VisibleSourceTypeLookupAttempts);
    if let Some(visible_source_bindings) = context.visible_source_bindings
        && let Some(canonical_path) = visible_source_bindings.get(&type_name)
        && let Some(declaration) = resolve_declaration_by_path(
            context.declaration_table,
            context.visible_declaration_ids,
            canonical_path,
        )
    {
        reject_bare_generic_type_name(type_name, canonical_path, location, context)?;
        return Ok(declaration.value.diagnostic_type.to_owned());
    }

    if let Some(declaration) = visible_declaration_by_name(
        context.declaration_table,
        context.visible_declaration_ids,
        type_name,
    ) {
        reject_bare_generic_type_name(type_name, &declaration.id, location, context)?;
        return Ok(declaration.value.diagnostic_type.to_owned());
    }

    // 4) Visible external types.
    if let Some(external_symbols) = context.visible_external_symbols
        && let Some(ExternalSymbolId::Type(type_id)) = external_symbols.get(&type_name)
    {
        return Ok(DataType::External { type_id: *type_id });
    }

    // 5) Traits are static contracts. They are valid in trait declarations,
    // conformances, and generic bounds, but never as ordinary value types.
    if let Some(trait_name) = visible_static_trait_name(type_name, context, string_table) {
        return Err(Box::new(CompilerDiagnostic::trait_name_used_as_type(
            trait_name,
            location.clone(),
        )));
    }

    // 6) Builtin type names that may still appear as named placeholders.
    if let Some(builtin_type) = builtin_named_type(type_name, string_table) {
        return Ok(builtin_type);
    }

    Err(Box::new(CompilerDiagnostic::unknown_type_name(
        type_name,
        location.to_owned(),
    )))
}

/// Find a visible trait name matching the supplied identifier.
///
/// WHAT: returns the canonical trait name when the identifier is either a user-visible trait or
///       a core compiler-owned trait such as the cast traits.
/// WHY: trait names must be reported as trait-name-as-type diagnostics even when they come from
///      different trait name sources.
fn visible_static_trait_name(
    type_name: StringId,
    context: &TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> Option<StringId> {
    if context
        .visible_trait_names
        .is_some_and(|visible_traits| visible_traits.contains_key(&type_name))
    {
        return Some(type_name);
    }

    let trait_environment = context.trait_environment?;

    if let Some(id) = trait_environment.core_trait_id_for_name(type_name, string_table) {
        return trait_environment.get(id).map(|definition| definition.name);
    }

    None
}

/// Resolve a namespace-qualified type name from the current context.
///
/// WHAT: looks up the namespace record, then resolves the member name to a source declaration,
///       external type, or namespace type/value misuse diagnostic. Supports multi-segment
///       paths for external package surfaces while keeping source and module public-surface
///       records shallow.
/// WHY: namespace-qualified type names follow a separate visibility path from bare names and
///      need explicit handling for the "value used as type" error shape.
pub(super) fn resolve_namespaced_type_from_context(
    path: &[StringId],
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
) -> TypeResolutionResult<DataType> {
    increment_ast_counter(AstCounter::VisibleTypeLookupAttempts);

    let Some(root_name) = path.first().copied() else {
        return Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                found: TokenKind::Eof,
            },
            location.to_owned(),
        )));
    };
    let final_name = path.last().copied().unwrap_or(root_name);

    let Some(visible_namespace_records) = context.visible_namespace_records else {
        return Err(Box::new(CompilerDiagnostic::unknown_type_name(
            final_name,
            location.to_owned(),
        )));
    };

    let Some(record) = visible_namespace_records.get(&root_name) else {
        return Err(Box::new(CompilerDiagnostic::unknown_type_name(
            final_name,
            location.to_owned(),
        )));
    };

    // Source and module public-surface namespace records remain shallow. Any attempt to traverse
    // deeper than one member in such a record must keep reporting the existing nested
    // traversal diagnostic, which integration fixtures already assert.
    if path.len() > 2 && matches!(record.record_source, NamespaceRecordSource::SourceFile(_)) {
        return Err(Box::new(CompilerDiagnostic::nested_dependency_traversal(
            root_name,
            location.to_owned(),
        )));
    }

    let mut current_record = record;

    // Walk every segment except the root and the final one. Each intermediate segment must
    // name a child namespace; any other slot produces a misuse diagnostic.
    for segment in path.iter().skip(1).take(path.len().saturating_sub(2)) {
        match lookup_namespace_member(current_record, *segment) {
            NamespaceMemberLookup::ChildNamespace(child_record) => {
                current_record = child_record;
            }
            NamespaceMemberLookup::Value(_) => {
                return Err(Box::new(CompilerDiagnostic::namespace_type_value_misuse(
                    *segment,
                    NamespaceTypeValueMisuseKind::Namespace,
                    NamespaceTypeValueMisuseKind::Value,
                    location.to_owned(),
                )));
            }
            NamespaceMemberLookup::Type => {
                return Err(Box::new(CompilerDiagnostic::namespace_type_value_misuse(
                    *segment,
                    NamespaceTypeValueMisuseKind::Namespace,
                    NamespaceTypeValueMisuseKind::Type,
                    location.to_owned(),
                )));
            }
            NamespaceMemberLookup::Missing => {
                return Err(Box::new(CompilerDiagnostic::unknown_type_name(
                    *segment,
                    location.to_owned(),
                )));
            }
        }
    }

    // The final segment must resolve to a type member of the namespace record we reached.
    increment_ast_counter(AstCounter::VisibleSourceTypeLookupAttempts);
    match current_record.type_members.get(&final_name) {
        Some(NamespaceTypeMember::SourceDeclaration(canonical_path)) => {
            if let Some(declaration) =
                resolve_declaration_by_path(context.declaration_table, None, canonical_path)
            {
                reject_bare_generic_type_name(final_name, &declaration.id, location, context)?;
                return Ok(declaration.value.diagnostic_type.to_owned());
            }
            Err(Box::new(CompilerDiagnostic::unknown_type_name(
                final_name,
                location.to_owned(),
            )))
        }
        Some(NamespaceTypeMember::ExternalSymbol(ExternalSymbolId::Type(type_id))) => {
            Ok(DataType::External { type_id: *type_id })
        }
        _ => match lookup_namespace_member(current_record, final_name) {
            NamespaceMemberLookup::Value(_) => {
                Err(Box::new(CompilerDiagnostic::namespace_type_value_misuse(
                    final_name,
                    NamespaceTypeValueMisuseKind::Type,
                    NamespaceTypeValueMisuseKind::Value,
                    location.to_owned(),
                )))
            }
            NamespaceMemberLookup::ChildNamespace(_) => {
                Err(Box::new(CompilerDiagnostic::namespace_type_value_misuse(
                    final_name,
                    NamespaceTypeValueMisuseKind::Type,
                    NamespaceTypeValueMisuseKind::Namespace,
                    location.to_owned(),
                )))
            }
            _ => Err(Box::new(CompilerDiagnostic::unknown_type_name(
                final_name,
                location.to_owned(),
            ))),
        },
    }
}

/// Resolve a generic base in source position.
///
/// WHAT: validates that a `GenericBaseType::Named` refers to a declared generic struct or choice,
///       and rejects value names, traits, aliases, external types, and builtins used with generic
///       arguments. Builtin generic types (`Collection`, `Map`) are returned unchanged.
/// WHY: generic application position has its own lookup rules: the base must name a generic
///      nominal declaration, and many valid type names are invalid as generic bases.
pub(super) fn resolve_generic_base_type(
    base: &GenericBaseType,
    arguments: &[DataType],
    location: &SourceLocation,
    context: &TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> TypeResolutionResult<GenericBaseType> {
    match base {
        GenericBaseType::Named(type_name) => {
            if let Some(reason) = invalid_carrier_type_syntax(*type_name, string_table) {
                return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
                    Some(*type_name),
                    reason,
                    location.to_owned(),
                )));
            }

            if let Some(generic_scope) = context.generic_parameters
                && generic_scope.contains_name(*type_name)
            {
                return Err(Box::new(CompilerDiagnostic::namespace_misuse(
                    *type_name,
                    NameNamespace::Type,
                    NameNamespace::Value,
                    location.to_owned(),
                )));
            }

            if let Some(visible_source_bindings) = context.visible_source_bindings
                && let Some(canonical_path) = visible_source_bindings.get(type_name)
            {
                return resolve_generic_base_path(
                    Some(*type_name),
                    canonical_path,
                    arguments,
                    location,
                    context,
                );
            }

            if let Some(declaration) = visible_declaration_by_name(
                context.declaration_table,
                context.visible_declaration_ids,
                *type_name,
            ) {
                return resolve_generic_base_path(
                    Some(*type_name),
                    &declaration.id,
                    arguments,
                    location,
                    context,
                );
            }

            if let Some(trait_name) = visible_static_trait_name(*type_name, context, string_table) {
                return Err(Box::new(CompilerDiagnostic::trait_name_used_as_type(
                    trait_name,
                    location.clone(),
                )));
            }

            if let Some(visible_aliases) = context.visible_type_aliases
                && visible_aliases.contains_key(type_name)
            {
                return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
                    Some(*type_name),
                    InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
                    location.to_owned(),
                )));
            }

            if let Some(external_symbols) = context.visible_external_symbols
                && matches!(
                    external_symbols.get(type_name),
                    Some(ExternalSymbolId::Type(_))
                )
            {
                return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
                    Some(*type_name),
                    InvalidGenericInstantiationReason::ExternalTypeArgumentsUnsupported,
                    location.to_owned(),
                )));
            }

            if builtin_named_type(*type_name, string_table).is_some() {
                return Err(Box::new(CompilerDiagnostic::namespace_misuse(
                    *type_name,
                    NameNamespace::Type,
                    NameNamespace::Value,
                    location.to_owned(),
                )));
            }

            Err(Box::new(CompilerDiagnostic::unknown_type_name(
                *type_name,
                location.to_owned(),
            )))
        }
        GenericBaseType::ResolvedNominal(path) => {
            resolve_generic_base_path(path.name(), path, arguments, location, context)
        }
        GenericBaseType::External(_) => {
            Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
                None,
                InvalidGenericInstantiationReason::ExternalTypeArgumentsUnsupported,
                location.to_owned(),
            )))
        }
        GenericBaseType::Builtin(BuiltinGenericType::Collection { fixed_capacity }) => {
            // Collection is the only builtin generic type allowed in source.
            // Its arguments are resolved separately by resolve_type.
            Ok(GenericBaseType::Builtin(BuiltinGenericType::Collection {
                fixed_capacity: *fixed_capacity,
            }))
        }
        GenericBaseType::Builtin(BuiltinGenericType::Map) => {
            // Map is allowed as a builtin generic type in source.
            // Its arguments are resolved separately by resolve_type.
            Ok(GenericBaseType::Builtin(BuiltinGenericType::Map))
        }
    }
}

/// Detect deferred public `Option` / `Result` syntax in generic position.
///
/// WHAT: returns the matching invalid-generic-instantiation reason when the name is `Option` or `Result`.
/// WHY: `Option of T` and `Result of T, E` are not Moth type syntax. Optional types use the
///      `T?` suffix and fallible functions declare a final `E!` error return slot.
fn invalid_carrier_type_syntax(
    type_name: StringId,
    string_table: &StringTable,
) -> Option<InvalidGenericInstantiationReason> {
    match string_table.resolve(type_name) {
        "Option" => Some(InvalidGenericInstantiationReason::OptionTypeSyntaxNotSupported),
        "Result" => Some(InvalidGenericInstantiationReason::ResultTypeSyntaxNotSupported),
        _ => None,
    }
}

/// Validate that a source declaration used as a bare type is not a generic struct/choice.
///
/// WHAT: when a generic nominal declaration is referenced without arguments, report a missing-
///       type-arguments diagnostic.
/// WHY: bare generic names like `Box` are not valid types; they must be applied as `Box of T`.
fn reject_bare_generic_type_name(
    visible_name: StringId,
    canonical_path: &InternedPath,
    location: &SourceLocation,
    context: &TypeResolutionContext<'_>,
) -> TypeResolutionResult<()> {
    let Some(metadata) = context
        .generic_declarations_by_path
        .and_then(|generic_declarations| generic_declarations.get(canonical_path))
    else {
        return Ok(());
    };

    if matches!(
        metadata.kind,
        GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
    ) {
        return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
            Some(visible_name),
            InvalidGenericInstantiationReason::MissingTypeArguments,
            location.to_owned(),
        )));
    }

    Ok(())
}

/// Resolve a generic base path to a declared generic struct or choice.
///
/// WHAT: checks that the canonical path names a generic struct/choice and that the argument count
///       matches the declaration-site parameter count, then returns `ResolvedNominal`.
/// WHY: this is the bridge from a visible name or already-resolved path to the canonical path
///      used by lazy generic instantiation.
fn resolve_generic_base_path(
    visible_name: Option<StringId>,
    canonical_path: &InternedPath,
    arguments: &[DataType],
    location: &SourceLocation,
    context: &TypeResolutionContext<'_>,
) -> TypeResolutionResult<GenericBaseType> {
    let Some(metadata) = context
        .generic_declarations_by_path
        .and_then(|generic_declarations| generic_declarations.get(canonical_path))
    else {
        return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
            visible_name,
            InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
            location.to_owned(),
        )));
    };

    if !matches!(
        metadata.kind,
        GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
    ) {
        return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
            visible_name,
            InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
            location.to_owned(),
        )));
    }

    let expected = metadata.parameters.len();
    let actual = arguments.len();
    if actual != expected {
        return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
            visible_name,
            InvalidGenericInstantiationReason::WrongArgumentCount {
                expected,
                found: actual,
            },
            location.to_owned(),
        )));
    }

    Ok(GenericBaseType::ResolvedNominal(canonical_path.to_owned()))
}

/// Fetch a declaration by canonical path, respecting the visible declaration id set.
fn resolve_declaration_by_path<'a>(
    declaration_table: &'a TopLevelDeclarationTable,
    visible_declaration_ids: Option<&FxHashSet<InternedPath>>,
    canonical_path: &InternedPath,
) -> Option<&'a Declaration> {
    declaration_table.get_visible_resolved_by_path(canonical_path, visible_declaration_ids)
}

/// Fetch a declaration by bare name, respecting the visible declaration id set.
fn visible_declaration_by_name<'a>(
    declaration_table: &'a TopLevelDeclarationTable,
    visible_declaration_ids: Option<&FxHashSet<InternedPath>>,
    name: StringId,
) -> Option<&'a Declaration> {
    declaration_table.get_visible_resolved_by_name(name, visible_declaration_ids)
}

/// Builtin scalar type names that may still appear as named placeholders.
fn builtin_named_type(type_name: StringId, string_table: &StringTable) -> Option<DataType> {
    match string_table.resolve(type_name) {
        "Int" => Some(DataType::Int),
        "Float" => Some(DataType::Float),
        "Bool" => Some(DataType::Bool),
        "String" => Some(DataType::StringSlice),
        "Char" => Some(DataType::Char),
        _ => None,
    }
}

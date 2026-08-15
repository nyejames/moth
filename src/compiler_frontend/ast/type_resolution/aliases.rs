//! Type alias lookup and alias-target re-resolution for AST type resolution.
//!
//! WHAT: finds visible type aliases by bare name or namespace-qualified name, then re-resolves
//!       the alias target so that the use site gets the correct semantic `TypeId` and diagnostic
//!       spelling.
//! WHY: alias resolution is a self-contained concern that touches visibility records, scope
//!      contexts, and parsed annotation recursion; keeping it in a focused module lets
//!      `resolve_type.rs` concentrate on the overall parsed-ref orchestration.
//!
//! This module owns:
//! - looking up a visible type alias by bare name or namespace-qualified name.
//! - deciding whether an alias target can reuse the already-resolved diagnostic spelling or
//!   must be re-resolved from its parsed source ref.
//! - building a source-file scope context for alias-target re-resolution.
//!
//! This module does NOT own:
//! - generic parameter resolution (lives in `generic_parameters.rs`).
//! - source declaration lookup, trait-name rejection, external type lookup, or generic-base
//!   validation (live in `lookup.rs`).
//! - generic nominal instantiation (lives in `generics.rs`).
//! - fixed collection capacity folding (lives in `collections.rs`).
//! - map nesting and key validation (lives in `maps.rs`).

use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult,
    context::{ResolvedTypeAnnotation, TypeResolutionContext, TypeResolutionContextInputs},
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::headers::binding_environment::NamespaceTypeMember;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::rc::Rc;

use super::resolve_type::{
    resolve_diagnostic_type_to_type_id_checked, resolve_parsed_type_annotation, resolve_type,
};

/// Look up a visible type alias by bare name.
///
/// WHAT: returns the alias's canonical path and its already-resolved annotation metadata when
///       the name resolves to a type alias in the current context.
/// WHY: callers need both the path (for source-file re-resolution) and the cached annotation
///      (for the diagnostic spelling and parsed source ref).
pub(super) fn visible_type_alias_annotation(
    name: StringId,
    context: &TypeResolutionContext<'_>,
) -> Option<(InternedPath, ResolvedTypeAnnotation)> {
    increment_ast_counter(AstCounter::VisibleTypeAliasLookupAttempts);

    let alias_path = context.visible_type_aliases?.get(&name)?;
    let annotation = context
        .resolved_type_aliases?
        .get(alias_path.local_path())?
        .clone();

    Some((alias_path.local_path().clone(), annotation))
}

/// Look up a visible type alias by namespace-qualified name.
///
/// WHAT: returns the alias's canonical path and its already-resolved annotation metadata when
///       the namespace record exposes a source declaration that is a resolved type alias.
pub(super) fn visible_namespaced_type_alias_annotation(
    namespace: StringId,
    name: StringId,
    context: &TypeResolutionContext<'_>,
) -> Option<(InternedPath, ResolvedTypeAnnotation)> {
    increment_ast_counter(AstCounter::VisibleTypeAliasLookupAttempts);

    let alias_path = context
        .visible_namespace_records?
        .get(&namespace)
        .and_then(|record| match record.type_members.get(&name) {
            Some(NamespaceTypeMember::SourceDeclaration(path)) => Some(path),
            _ => None,
        })?;
    let annotation = context
        .resolved_type_aliases?
        .get(alias_path.local_path())?
        .clone();

    Some((alias_path.local_path().clone(), annotation))
}

/// Re-resolve a type alias target so the use site gets the right semantic identity.
///
/// WHAT: alias targets are resolved once at the declaration site and cached as a
///       `ResolvedTypeAnnotation`. Most aliases can reuse that cached diagnostic spelling.
///       Aliases whose parsed target contains fixed collection capacity must be re-resolved
///       through the alias declaration's source-file visibility, because the capacity value
///       may refer to constants that are only visible in the alias's declaring file.
/// WHY: fixed-capacity collection types (`{N T}`) encode their capacity in the canonical
///      `TypeId`. The capacity value is folded against the declaration-site scope, so a
///      use site in another file cannot fold the same capacity without the alias file's
///      visibility. Re-resolving through the source-file scope gives the same answer the
///      declaration site already computed.
///
/// Body-local declarations from the use site are explicitly cleared during re-resolution
/// because type aliases are top-level metadata and must not be influenced by local variables
/// or declarations where the alias is used.
pub(super) fn resolve_alias_annotation(
    alias_path: InternedPath,
    annotation: ResolvedTypeAnnotation,
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<ResolvedTypeAnnotation> {
    if !parsed_ref_contains_fixed_capacity(&annotation.source_ref) {
        let diagnostic_type =
            resolve_type(&annotation.diagnostic_type, location, context, string_table)?;
        let type_id = if matches!(diagnostic_type, DataType::Inferred) {
            None
        } else {
            Some(resolve_diagnostic_type_to_type_id_checked(
                &diagnostic_type,
                context.type_environment,
                location,
            )?)
        };

        return Ok(ResolvedTypeAnnotation {
            source_ref: annotation.source_ref,
            diagnostic_type,
            type_id,
        });
    }

    let Some(alias_scope_context) = alias_scope_context(&alias_path, scope_context) else {
        return resolve_parsed_type_annotation(
            annotation.source_ref,
            location,
            context,
            string_table,
            scope_context,
        );
    };

    let mut alias_context = TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
        declaration_table: context.declaration_table,
        visible_declaration_ids: alias_scope_context.visible_declaration_ids.as_ref(),
        visible_external_symbols: alias_scope_context
            .file_visibility
            .as_ref()
            .map(|visibility| &visibility.visible_external_symbols),
        visible_source_bindings: alias_scope_context
            .file_visibility
            .as_ref()
            .map(|visibility| &visibility.visible_source_names),
        visible_type_aliases: alias_scope_context
            .file_visibility
            .as_ref()
            .map(|visibility| &visibility.visible_type_alias_names),
        resolved_type_aliases: context.resolved_type_aliases,
        generic_declarations_by_path: context.generic_declarations_by_path,
        resolved_struct_fields_by_path: context.resolved_struct_fields_by_path,
        type_environment: context.type_environment,
        visible_namespace_records: alias_scope_context
            .file_visibility
            .as_ref()
            .map(|visibility| &visibility.visible_namespace_records),
        trait_environment: context.trait_environment,
        trait_evidence_environment: context.trait_evidence_environment,
        visible_trait_names: alias_scope_context
            .file_visibility
            .as_ref()
            .map(|visibility| &visibility.visible_trait_names),
    });

    resolve_parsed_type_annotation(
        annotation.source_ref,
        location,
        &mut alias_context,
        string_table,
        Some(&alias_scope_context),
    )
}

/// Detect whether a parsed type reference contains fixed collection capacity syntax.
///
/// WHAT: returns true if any subexpression of `source_ref` is a collection with an explicit
///       fixed capacity (`{N T}`) or a nested type that contains one.
/// WHY: fixed capacity must be folded against the declaration-site scope, so this predicate
///      decides whether an alias target needs full source-file re-resolution.
fn parsed_ref_contains_fixed_capacity(source_ref: &ParsedTypeRef) -> bool {
    match source_ref {
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            ..
        } => fixed_capacity.is_some() || parsed_ref_contains_fixed_capacity(element),
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            parsed_ref_contains_fixed_capacity(base)
                || arguments.iter().any(parsed_ref_contains_fixed_capacity)
        }
        ParsedTypeRef::Optional { inner, .. } => parsed_ref_contains_fixed_capacity(inner),
        ParsedTypeRef::Result { ok, err, .. } => {
            parsed_ref_contains_fixed_capacity(ok) || parsed_ref_contains_fixed_capacity(err)
        }
        _ => false,
    }
}

/// Build a scope context for re-resolving an alias target in its declaring file.
///
/// WHAT: looks up the source file that owns the alias declaration, then constructs a
///       scope context with that file's visibility and with body-local declarations cleared.
/// WHY: fixed-capacity collection types may refer to file-private constants, and aliases
///      are top-level metadata that must not see body-local declarations from the use site.
fn alias_scope_context(
    alias_path: &InternedPath,
    scope_context: Option<&ScopeContext>,
) -> Option<ScopeContext> {
    let scope_context = scope_context?;
    let source_file = scope_context
        .shared
        .lookups
        .module_symbols
        .canonical_source_by_symbol_path
        .get(alias_path)?;
    let visibility = scope_context
        .shared
        .lookups
        .binding_environment
        .visibility_for(source_file)
        .ok()?
        .clone();
    let mut alias_scope = scope_context
        .clone()
        .with_file_visibility(Rc::new(visibility))
        .with_source_file_scope(source_file.clone());

    // Type aliases are top-level metadata. Re-resolving an alias target must not see
    // body-local declarations from the use site.
    alias_scope.set_local_declarations(Vec::new());

    Some(alias_scope)
}

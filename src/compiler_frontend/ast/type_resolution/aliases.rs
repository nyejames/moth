//! Type alias lookup and completed alias projection for AST type resolution.
//!
//! WHAT: finds visible type aliases by bare name or namespace-qualified name and projects their
//!       completed declaration-site metadata into a use-site annotation.
//! WHY: aliases are resolved once while their declaration file is being built. Keeping lookup
//!      here lets `resolve_type.rs` concentrate on the overall parsed-ref orchestration without
//!      reopening a declaring module's mutable type environment.
//!
//! This module owns:
//! - looking up a visible type alias by bare name or namespace-qualified name.
//! - projecting a completed alias into a use-site annotation with its required `TypeId`.
//!
//! This module does NOT own:
//! - alias target resolution (lives in `environment/type_aliases.rs`).
//! - generic parameter resolution (lives in `generic_parameters.rs`).
//! - source declaration lookup, trait-name rejection, external type lookup, or generic-base
//!   validation (live in `lookup.rs`).
//! - generic nominal instantiation (lives in `generics.rs`).
//! - map nesting and key validation (lives in `maps.rs`).

use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult,
    context::{ResolvedTypeAlias, ResolvedTypeAnnotation, TypeResolutionContext},
};
use crate::compiler_frontend::headers::binding_environment::NamespaceTypeMember;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
/// Look up a visible type alias by bare name.
///
/// WHAT: returns the alias's canonical path and completed metadata when the name resolves to a
/// type alias in the current context.
/// WHY: aliases are published only after their target `TypeId` is complete, so every use-site
/// projection can consume the required identity without retrying resolution.
pub(super) fn visible_type_alias_annotation(
    name: StringId,
    context: &TypeResolutionContext<'_>,
) -> Option<(InternedPath, ResolvedTypeAlias)> {
    increment_ast_counter(AstCounter::VisibleTypeAliasLookupAttempts);

    let alias_path = context.visible_type_aliases?.get(&name)?;
    let alias = context
        .resolved_type_aliases?
        .get(alias_path.local_path())?
        .clone();

    Some((alias_path.local_path().clone(), alias))
}

/// Look up a visible type alias by namespace-qualified name.
///
/// WHAT: returns the alias's canonical path and completed metadata when the namespace record
/// exposes a source declaration that is a resolved type alias.
pub(super) fn visible_namespaced_type_alias_annotation(
    namespace: StringId,
    name: StringId,
    context: &TypeResolutionContext<'_>,
) -> Option<(InternedPath, ResolvedTypeAlias)> {
    increment_ast_counter(AstCounter::VisibleTypeAliasLookupAttempts);

    let alias_path = context
        .visible_namespace_records?
        .get(&namespace)
        .and_then(|record| match record.type_members.get(&name) {
            Some(NamespaceTypeMember::SourceDeclaration(path)) => Some(path),
            _ => None,
        })?;
    let alias = context
        .resolved_type_aliases?
        .get(alias_path.local_path())?
        .clone();

    Some((alias_path.local_path().clone(), alias))
}

/// Project completed alias metadata into the general annotation result used by type resolution.
///
/// The alias table itself stores the required target identity. This conversion is deliberately
/// kept at the use site because other annotation contexts may still represent inference.
pub(super) fn resolve_alias_annotation(
    alias: ResolvedTypeAlias,
) -> TypeResolutionResult<ResolvedTypeAnnotation> {
    Ok(ResolvedTypeAnnotation {
        diagnostic_type: alias.diagnostic_type,
        type_id: Some(alias.target_type_id),
    })
}

//! Type alias lookup by visible canonical identity.
//!
//! WHAT: finds visible aliases by bare or namespace-qualified name and returns their completed
//!       canonical `TypeId`.
//! WHY: aliases are resolved once while their declaration file is being built, so use-site lookup
//!      can consume declaration-site identity without reopening a mutable type environment.
//!
//! This module owns:
//! - returning the resolved target identity from completed declaration-site metadata.
//!
//! This module does NOT own:
//! - alias target resolution (lives in `environment/type_aliases.rs`).
//! - generic parameter resolution (lives in `generic_parameters.rs`).
//! - source declaration lookup, trait-name rejection, external type lookup, or generic-base
//!   validation (live in `lookup.rs`).
//! - generic nominal instantiation (lives in `generics.rs`).
//! - map nesting and key validation (lives in `maps.rs`).

use crate::compiler_frontend::ast::type_resolution::context::{
    ResolvedTypeAlias, TypeResolutionContext,
};
use crate::compiler_frontend::headers::binding_environment::NamespaceTypeMember;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringId;
/// Look up a visible type alias by bare name.
///
/// WHAT: returns completed metadata when the name resolves to a type alias in the current
/// context.
/// WHY: aliases are published only after their target `TypeId` is complete, so every use-site
/// projection can consume the required identity without retrying resolution.
pub(super) fn visible_type_alias_annotation<'a>(
    name: StringId,
    context: &'a TypeResolutionContext<'_>,
) -> Option<&'a ResolvedTypeAlias> {
    increment_ast_counter(AstCounter::VisibleTypeAliasLookupAttempts);

    let alias_path = context.visible_type_aliases?.get(&name)?;
    context.resolved_type_aliases?.get(alias_path.local_path())
}

/// Look up a visible type alias by namespace-qualified name.
///
/// WHAT: returns completed metadata when the namespace record exposes a source declaration that
/// is a resolved type alias.
pub(super) fn visible_namespaced_type_alias_annotation<'a>(
    namespace: StringId,
    name: StringId,
    context: &'a TypeResolutionContext<'_>,
) -> Option<&'a ResolvedTypeAlias> {
    increment_ast_counter(AstCounter::VisibleTypeAliasLookupAttempts);

    let alias_path = context
        .visible_namespace_records?
        .get(&namespace)
        .and_then(|record| match record.type_members.get(&name) {
            Some(NamespaceTypeMember::SourceDeclaration(path)) => Some(path),
            _ => None,
        })?;
    context.resolved_type_aliases?.get(alias_path.local_path())
}

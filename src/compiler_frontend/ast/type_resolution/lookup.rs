//! Source-visible type-name lookup and trait-name rejection for AST type resolution.
//!
//! WHAT: resolves source-authored bare and namespace-qualified type names to the declarations,
//!       aliases, external types, generic parameters, builtins, and traits visible in the current
//!       type-resolution context. It also rejects names that are values, traits, or generic bases
//!       used in positions where a concrete type is required.
//! WHY: name lookup is a self-contained concern that touches visibility records, declaration
//!      tables, namespace records, generic scopes, and trait environments; keeping it in a focused
//!      module lets `resolve_type.rs` concentrate on parsed-ref orchestration and canonical type
//!      identity.
//!
//! This module owns:
//! - resolving bare named types to visible declarations, aliases, external types, generic
//!   parameters, builtins, or trait-name rejections.
//! - resolving namespace-qualified types to visible namespace type members or namespace
//!   type/value misuse diagnostics.
//! - resolving generic application bases to declared generic structs/choices and rejecting other
//!   generic-base misuses.
//! - rejecting bare generic type names used without type arguments.
//! - rejecting trait names used as ordinary types.
//!
//! This module does NOT own:
//! - `TypeResolutionContext` construction or result types (live in `context.rs`).
//! - completed alias lookup and use-site projection (lives in `aliases.rs`).
//! - lazy generic struct/choice instance materialization or bound-evidence validation
//!   (lives in `generics.rs`).
//! - collection capacity folding or map key validation (live in `collections.rs` and `maps.rs`).

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult, aliases,
    context::{TypeResolutionContext, TypeResolutionFailureKind},
};
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    generics_traits_api, names_declarations, project_config,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidGenericInstantiationReason, InvalidTypeAnnotationReason, NameNamespace,
    NamespaceTypeValueMisuseKind, TypeAnnotationContext, UnknownTypeNameSite,
};
use crate::compiler_frontend::datatypes::display::{
    BOOL_TYPE_SPELLING, CHAR_TYPE_SPELLING, STRING_TYPE_SPELLING,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterScope;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::{
    FileVisibility, NamespaceMemberLookup, NamespaceRecordSource, NamespaceTypeMember,
    SourceDeclarationTarget, lookup_namespace_member,
};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;

fn unknown_type_name(
    name: StringId,
    site: UnknownTypeNameSite,
    span: Option<SourceSpan>,
    candidates: Vec<StringId>,
    failure_kind: &mut TypeResolutionFailureKind,
) -> DiagnosticDraft {
    *failure_kind = TypeResolutionFailureKind::UnknownTypeName;
    names_declarations::unknown_type_name(name, site, candidates, span)
}

/// Visible type-name maps shared by unknown-type suggestion producers.
///
/// WHAT: the file-local visibility tables plus optional generic parameters.
/// WHY: alias and conformance sites have `FileVisibility` rather than a full
/// type-resolution context, so one collector still owns the candidate set.
pub(crate) struct VisibleTypeNameSources<'a> {
    pub visible_source_bindings: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub visible_declaration_ids: Option<&'a FxHashSet<PathId>>,
    pub visible_type_aliases: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub visible_external_symbols: Option<&'a FxHashMap<StringId, ExternalSymbolId>>,
    pub generic_parameters: Option<&'a GenericParameterScope>,
}

impl<'a> VisibleTypeNameSources<'a> {
    pub(crate) fn from_context(context: &'a TypeResolutionContext<'_>) -> Self {
        Self {
            visible_source_bindings: context.visible_source_bindings,
            visible_declaration_ids: context.visible_declaration_ids.map(Arc::as_ref),
            visible_type_aliases: context.visible_type_aliases,
            visible_external_symbols: context.visible_external_symbols,
            generic_parameters: context.generic_parameters,
        }
    }

    pub(crate) fn from_file_visibility(visibility: &'a FileVisibility) -> Self {
        Self {
            visible_source_bindings: Some(&visibility.visible_source_names),
            visible_declaration_ids: Some(visibility.visible_declaration_paths.as_ref()),
            visible_type_aliases: Some(&visibility.visible_type_alias_names),
            visible_external_symbols: Some(&visibility.visible_external_symbols),
            generic_parameters: None,
        }
    }

    pub(crate) fn same_file_nominal_types(visibility: &'a FileVisibility) -> Self {
        Self {
            visible_source_bindings: Some(&visibility.visible_source_names),
            visible_declaration_ids: Some(visibility.visible_declaration_paths.as_ref()),
            visible_type_aliases: None,
            visible_external_symbols: None,
            generic_parameters: None,
        }
    }
}

/// Collect visible type-name candidates for an unknown-type diagnostic.
///
/// WHAT: gathers interned names from visibility tables only, gated by visible
/// declaration ids and an `is_nominal_type` filter for source bindings.
/// WHY: iterating `declaration_table` wholesale would leak private and
/// unimported types; hash-map order must not affect the chosen suggestion.
pub(crate) fn collect_visible_type_name_candidates(
    unknown_name: StringId,
    sources: VisibleTypeNameSources<'_>,
    is_nominal_type: impl Fn(&PathId) -> bool,
) -> Vec<StringId> {
    let mut names = FxHashSet::default();

    if let Some(bindings) = sources.visible_source_bindings {
        for (name, target) in bindings {
            if *name == unknown_name {
                continue;
            }
            if let Some(visible_ids) = sources.visible_declaration_ids
                && !visible_ids.contains(target.local_path())
            {
                continue;
            }
            if !is_nominal_type(target.local_path()) {
                continue;
            }
            names.insert(*name);
        }
    }

    if let Some(aliases) = sources.visible_type_aliases {
        for name in aliases.keys() {
            if *name != unknown_name {
                names.insert(*name);
            }
        }
    }

    if let Some(externals) = sources.visible_external_symbols {
        for (name, symbol) in externals {
            if *name != unknown_name && matches!(symbol, ExternalSymbolId::Type(_)) {
                names.insert(*name);
            }
        }
    }

    if let Some(generic_parameters) = sources.generic_parameters {
        for name in generic_parameters.parameter_names() {
            if name != unknown_name {
                names.insert(name);
            }
        }
    }

    let mut candidates: Vec<StringId> = names.into_iter().collect();
    candidates.sort_unstable_by_key(|name| name.index());
    candidates
}

/// True when `path` names a visible struct or choice shell.
pub(crate) fn declaration_is_nominal_type(
    declaration_table: &TopLevelDeclarationTable,
    path: &PathId,
) -> bool {
    declaration_table
        .get_visible_resolved_by_path(path, None)
        .is_some_and(|declaration| {
            matches!(
                declaration.value,
                DeclarationValue::Shell(DeclarationShell::Struct | DeclarationShell::Choice)
            )
        })
}

impl TypeResolutionContext<'_> {
    pub(crate) fn visible_type_name_candidates(&self, unknown_name: StringId) -> Vec<StringId> {
        if self.skip_unknown_type_name_candidates {
            return Vec::new();
        }
        let mut candidates = collect_visible_type_name_candidates(
            unknown_name,
            VisibleTypeNameSources::from_context(self),
            |path| declaration_is_nominal_type(self.declaration_table, path),
        );
        if let Some(excluded) = self.excluded_unknown_type_name {
            candidates.retain(|name| *name != excluded);
        }
        candidates
    }
}

/// Resolve a bare named type from the current type-resolution context.
///
/// WHAT: searches generic parameters, visible type aliases, visible source declarations,
///       visible external types, visible traits, and builtin type names in order, returning
///       the canonical `TypeId` when one is registered or an appropriate diagnostic.
/// WHY: this is the single lookup path for source-authored bare type names; centralizing it keeps
///      the priority order between parameters, aliases, declarations, and builtins explicit.
pub(super) fn resolve_named_type_from_context(
    type_name: StringId,
    span: Option<SourceSpan>,
    context: &mut TypeResolutionContext<'_>,
    string_table: &StringTable,
    failure_kind: &mut TypeResolutionFailureKind,
) -> TypeResolutionResult<Option<TypeId>> {
    if let Some(generic_scope) = context.generic_parameters
        && let Some(parameter) = generic_scope.resolve(type_name)
    {
        increment_ast_counter(AstCounter::VisibleTypeLookupAttempts);

        if let Some(canonical_id) = parameter.canonical_id
            && let Some(substitutions) = context.generic_substitutions
            && let Some(concrete_type_id) = substitutions.get(&canonical_id).copied()
        {
            return Ok(Some(concrete_type_id));
        }

        let Some(canonical_id) = parameter.canonical_id else {
            return Ok(None);
        };
        return Ok(Some(
            context
                .type_environment
                .intern_generic_parameter(canonical_id, parameter.name),
        ));
    }

    // 2) Visible type aliases.
    if let Some(alias) = aliases::visible_type_alias_annotation(type_name, context) {
        return Ok(Some(alias.target_type_id));
    }

    // Alias hits return early, so only lookups that reach here count as visible-type attempts.
    increment_ast_counter(AstCounter::VisibleTypeLookupAttempts);

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
        reject_callable_type_name(type_name, declaration, span)?;
        reject_bare_generic_type_name(type_name, canonical_path, span, context)?;
        return Ok(registered_declaration_type_id(declaration, context));
    }

    if let Some(declaration) = visible_declaration_by_name(
        context.declaration_table,
        context.visible_declaration_ids,
        type_name,
    ) {
        reject_callable_type_name(type_name, declaration, span)?;
        reject_bare_generic_type_name(type_name, &declaration.id, span, context)?;
        return Ok(registered_declaration_type_id(declaration, context));
    }

    // 4) Visible external types.
    if let Some(external_symbols) = context.visible_external_symbols
        && let Some(ExternalSymbolId::Type(type_id)) = external_symbols.get(&type_name)
    {
        return Ok(Some(context.type_environment.intern_external(*type_id)));
    }

    // 5) Traits are static contracts. They are valid in trait declarations,
    // conformances, and generic bounds, but never as ordinary value types.
    if let Some(trait_name) = visible_static_trait_name(type_name, context, string_table) {
        return Err(generics_traits_api::trait_name_used_as_type(
            trait_name, span,
        ));
    }

    // 6) Builtin type names that may still appear as named placeholders.
    if let Some(builtin_type) =
        builtin_named_type(type_name, string_table, context.type_environment)
    {
        return Ok(Some(builtin_type));
    }

    Err(unknown_type_name(
        type_name,
        UnknownTypeNameSite::TypeAnnotation,
        span,
        context.visible_type_name_candidates(type_name),
        failure_kind,
    ))
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
    span: Option<SourceSpan>,
    context: &mut TypeResolutionContext<'_>,
    failure_kind: &mut TypeResolutionFailureKind,
) -> TypeResolutionResult<Option<TypeId>> {
    let Some(root_name) = path.first().copied() else {
        return Err(names_declarations::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                found: DiagnosticToken::from_static_tag(TokenTag::EOF),
            },
            span,
        ));
    };
    let final_name = path.last().copied().unwrap_or(root_name);

    // Source type aliases are visible only through shallow namespace records.
    if path.len() == 2
        && let Some(alias) =
            aliases::visible_namespaced_type_alias_annotation(root_name, final_name, context)
    {
        return Ok(Some(alias.target_type_id));
    }

    // Qualified alias hits likewise do not enter the visible-member lookup.
    increment_ast_counter(AstCounter::VisibleTypeLookupAttempts);

    let Some(visible_namespace_records) = context.visible_namespace_records else {
        return Err(unknown_type_name(
            final_name,
            UnknownTypeNameSite::NamespaceMember,
            span,
            Vec::new(),
            failure_kind,
        ));
    };

    let Some(record) = visible_namespace_records.get(&root_name) else {
        return Err(unknown_type_name(
            final_name,
            UnknownTypeNameSite::NamespaceMember,
            span,
            Vec::new(),
            failure_kind,
        ));
    };

    // Source and module public-surface namespace records remain shallow. Any attempt to traverse
    // deeper than one member in such a record must keep reporting the existing nested
    // traversal diagnostic, which integration fixtures already assert.
    if path.len() > 2 && matches!(record.record_source, NamespaceRecordSource::SourceFile(_)) {
        return Err(project_config::nested_dependency_traversal(root_name, span));
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
                return Err(project_config::namespace_type_value_misuse(
                    *segment,
                    NamespaceTypeValueMisuseKind::Namespace,
                    NamespaceTypeValueMisuseKind::Value,
                    span,
                ));
            }
            NamespaceMemberLookup::Type => {
                return Err(project_config::namespace_type_value_misuse(
                    *segment,
                    NamespaceTypeValueMisuseKind::Namespace,
                    NamespaceTypeValueMisuseKind::Type,
                    span,
                ));
            }
            NamespaceMemberLookup::Missing => {
                return Err(unknown_type_name(
                    *segment,
                    UnknownTypeNameSite::NamespaceMember,
                    span,
                    Vec::new(),
                    failure_kind,
                ));
            }
        }
    }

    // The final segment must resolve to a type member of the namespace record we reached.
    increment_ast_counter(AstCounter::VisibleSourceTypeLookupAttempts);
    match current_record.type_members.get(&final_name) {
        Some(NamespaceTypeMember::SourceDeclaration(canonical_path)) => {
            if let Some(declaration) = resolve_declaration_by_path(
                context.declaration_table,
                None,
                canonical_path.local_path(),
            ) {
                reject_callable_type_name(final_name, declaration, span)?;
                reject_bare_generic_type_name(final_name, &declaration.id, span, context)?;
                return Ok(registered_declaration_type_id(declaration, context));
            }
            Err(unknown_type_name(
                final_name,
                UnknownTypeNameSite::NamespaceMember,
                span,
                Vec::new(),
                failure_kind,
            ))
        }
        Some(NamespaceTypeMember::ExternalSymbol(ExternalSymbolId::Type(type_id))) => {
            Ok(Some(context.type_environment.intern_external(*type_id)))
        }
        _ => match lookup_namespace_member(current_record, final_name) {
            NamespaceMemberLookup::Value(_) => Err(project_config::namespace_type_value_misuse(
                final_name,
                NamespaceTypeValueMisuseKind::Type,
                NamespaceTypeValueMisuseKind::Value,
                span,
            )),
            NamespaceMemberLookup::ChildNamespace(_) => {
                Err(project_config::namespace_type_value_misuse(
                    final_name,
                    NamespaceTypeValueMisuseKind::Type,
                    NamespaceTypeValueMisuseKind::Namespace,
                    span,
                ))
            }
            _ => Err(unknown_type_name(
                final_name,
                UnknownTypeNameSite::NamespaceMember,
                span,
                Vec::new(),
                failure_kind,
            )),
        },
    }
}

/// Resolve a generic base in source position.
///
/// WHAT: validates that a named base refers to a declared generic struct or choice and returns its
///       canonical path, rejecting value names, traits, aliases, external types and builtins used
///       with generic arguments.
/// WHY: generic application position has its own lookup rules: the base must name a generic
///      nominal declaration, and many valid type names are invalid as generic bases.
pub(super) fn resolve_generic_base_type(
    type_name: StringId,
    argument_count: usize,
    span: Option<SourceSpan>,
    context: &TypeResolutionContext<'_>,
    string_table: &StringTable,
    failure_kind: &mut TypeResolutionFailureKind,
) -> TypeResolutionResult<PathId> {
    if let Some(reason) = invalid_carrier_type_syntax(type_name, string_table) {
        return Err(generics_traits_api::invalid_generic_instantiation(
            Some(type_name),
            reason,
            span,
        ));
    }

    if let Some(generic_scope) = context.generic_parameters
        && generic_scope.contains_name(type_name)
    {
        return Err(names_declarations::namespace_misuse(
            type_name,
            NameNamespace::Type,
            NameNamespace::Value,
            span,
        ));
    }

    if let Some(visible_source_bindings) = context.visible_source_bindings
        && let Some(canonical_path) = visible_source_bindings.get(&type_name)
    {
        return resolve_generic_base_path(
            Some(type_name),
            canonical_path,
            argument_count,
            span,
            context,
        );
    }

    if let Some(declaration) = visible_declaration_by_name(
        context.declaration_table,
        context.visible_declaration_ids,
        type_name,
    ) {
        return resolve_generic_base_path(
            Some(type_name),
            &declaration.id,
            argument_count,
            span,
            context,
        );
    }

    if let Some(trait_name) = visible_static_trait_name(type_name, context, string_table) {
        return Err(generics_traits_api::trait_name_used_as_type(
            trait_name, span,
        ));
    }

    if let Some(visible_aliases) = context.visible_type_aliases
        && visible_aliases.contains_key(&type_name)
    {
        return Err(generics_traits_api::invalid_generic_instantiation(
            Some(type_name),
            InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
            span,
        ));
    }

    if let Some(external_symbols) = context.visible_external_symbols
        && matches!(
            external_symbols.get(&type_name),
            Some(ExternalSymbolId::Type(_))
        )
    {
        return Err(generics_traits_api::invalid_generic_instantiation(
            Some(type_name),
            InvalidGenericInstantiationReason::ExternalTypeArgumentsUnsupported,
            span,
        ));
    }

    if builtin_named_type(type_name, string_table, context.type_environment).is_some() {
        return Err(names_declarations::namespace_misuse(
            type_name,
            NameNamespace::Type,
            NameNamespace::Value,
            span,
        ));
    }

    Err(unknown_type_name(
        type_name,
        UnknownTypeNameSite::TypeAnnotation,
        span,
        context.visible_type_name_candidates(type_name),
        failure_kind,
    ))
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

/// Callable shells carry no value type and must be rejected before their canonical
/// `TypeId` can be mistaken for an ordinary type.
fn reject_callable_type_name(
    visible_name: StringId,
    declaration: &Declaration,
    span: Option<SourceSpan>,
) -> TypeResolutionResult<()> {
    if matches!(
        declaration.value,
        DeclarationValue::Shell(DeclarationShell::Function { .. })
    ) {
        return Err(project_config::namespace_type_value_misuse(
            visible_name,
            NamespaceTypeValueMisuseKind::Type,
            NamespaceTypeValueMisuseKind::Value,
            span,
        ));
    }

    Ok(())
}

/// Return a source declaration's canonical type only when its identity is registered here.
fn registered_declaration_type_id(
    declaration: &Declaration,
    context: &TypeResolutionContext<'_>,
) -> Option<TypeId> {
    let type_id = declaration.type_id;
    if type_id == builtin_type_ids::NONE {
        return None;
    }
    context.type_environment.get(type_id)?;
    match &declaration.value {
        DeclarationValue::Shell(DeclarationShell::Struct | DeclarationShell::Choice) => {
            let nominal_id = context
                .type_environment
                .nominal_id_for_path(&declaration.id)?;
            (context.type_environment.type_id_for_nominal_id(nominal_id) == Some(type_id))
                .then_some(type_id)
        }
        // Completed aliases resolve through their `target_type_id` metadata. An alias shell
        // without that metadata does not yet carry semantic type identity.
        DeclarationValue::Shell(DeclarationShell::TypeAlias) => None,
        _ => Some(type_id),
    }
}

/// Validate that a source declaration used as a bare type is not a generic struct/choice.
///
/// WHAT: when a generic nominal declaration is referenced without arguments, report a missing-
///       type-arguments diagnostic.
/// WHY: bare generic names like `Box` are not valid types; they must be applied as `Box of T`.
fn reject_bare_generic_type_name(
    visible_name: StringId,
    canonical_path: &PathId,
    span: Option<SourceSpan>,
    context: &TypeResolutionContext<'_>,
) -> TypeResolutionResult<()> {
    let Some(kind) = context
        .generic_declarations_by_path
        .and_then(|generic_declarations| generic_declarations.get(canonical_path))
    else {
        return Ok(());
    };

    if matches!(
        kind,
        GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
    ) {
        return Err(generics_traits_api::invalid_generic_instantiation(
            Some(visible_name),
            InvalidGenericInstantiationReason::MissingTypeArguments,
            span,
        ));
    }

    Ok(())
}

/// Validate a canonical generic nominal base path.
///
/// WHAT: checks that the canonical path names a generic struct/choice and that the argument count
///       matches the declaration-site parameter count, then returns the path.
/// WHY: this is the bridge from a visible name or already-resolved path to the canonical path
///      used by lazy generic instantiation.
fn resolve_generic_base_path(
    visible_name: Option<StringId>,
    canonical_path: &PathId,
    argument_count: usize,
    span: Option<SourceSpan>,
    context: &TypeResolutionContext<'_>,
) -> TypeResolutionResult<PathId> {
    let Some(kind) = context
        .generic_declarations_by_path
        .and_then(|generic_declarations| generic_declarations.get(canonical_path))
    else {
        return Err(generics_traits_api::invalid_generic_instantiation(
            visible_name,
            InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
            span,
        ));
    };

    if !matches!(
        kind,
        GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
    ) {
        return Err(generics_traits_api::invalid_generic_instantiation(
            visible_name,
            InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
            span,
        ));
    }

    let expected = context
        .type_environment
        .canonical_parameters_for_nominal(canonical_path)
        .expect("generic nominal registration must include canonical parameters")
        .len();
    let actual = argument_count;
    if actual != expected {
        return Err(generics_traits_api::invalid_generic_instantiation(
            visible_name,
            InvalidGenericInstantiationReason::WrongArgumentCount {
                expected,
                found: actual,
            },
            span,
        ));
    }

    Ok(*canonical_path)
}

/// Fetch a declaration by canonical path, respecting the visible declaration id set.
fn resolve_declaration_by_path<'a>(
    declaration_table: &'a TopLevelDeclarationTable,
    visible_declaration_ids: Option<&Arc<FxHashSet<PathId>>>,
    canonical_path: &PathId,
) -> Option<&'a Declaration> {
    declaration_table
        .get_visible_resolved_by_path(canonical_path, visible_declaration_ids.map(Arc::as_ref))
}

/// Fetch a declaration by bare name, respecting the visible declaration id set.
fn visible_declaration_by_name<'a>(
    declaration_table: &'a TopLevelDeclarationTable,
    visible_declaration_ids: Option<&Arc<FxHashSet<PathId>>>,
    name: StringId,
) -> Option<&'a Declaration> {
    declaration_table.get_visible_resolved_by_name(name, visible_declaration_ids.map(Arc::as_ref))
}

/// Builtin scalar type names that may still appear as named placeholders.
fn builtin_named_type(
    type_name: StringId,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> Option<TypeId> {
    let resolved = string_table.resolve(type_name);
    let builtins = type_environment.builtins();
    if resolved == BOOL_TYPE_SPELLING {
        Some(builtins.bool)
    } else if resolved == STRING_TYPE_SPELLING {
        Some(builtins.string)
    } else if resolved == CHAR_TYPE_SPELLING {
        Some(builtins.char)
    } else {
        // Fixed-scalar spellings resolve through their single spelling owner.
        Some(builtin_type_ids::fixed_scalar(FixedScalar::from_name(
            resolved,
        )?))
    }
}

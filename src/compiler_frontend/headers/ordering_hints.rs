//! Header-stage local declaration-ordering hint collection.
//!
//! WHAT: converts type references from declaration shells into conservative local
//! declaration-ordering hints retained before provider binding.
//! WHY: syntax preparation records the dependency spelling or same-file spelling uniformly without
//! knowing which dependencies are source graph participants versus virtual or provider bindings.
//! Binding canonicalizes or drops dependency-spelled hints; Stage 3 resolves retained local hints
//! into sortable graph edges.

use crate::compiler_frontend::builtins::error_type::{
    is_reserved_builtin_symbol, is_reserved_parameter_contract_symbol,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::{CapacityResource, CompilerStage};
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    ParsedNamedTypeReference, collect_capacity_references_in_parsed_ref,
    for_each_named_type_in_parsed_ref,
};
use crate::compiler_frontend::headers::binding_environment::{
    HeaderBindingEnvironment, ResolvedQualifiedTarget,
};
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::qualified_references::push_const_required_expression_ranges;
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::headers::types::{
    DependencySelection, Header, HeaderBuildContext, HeaderParseFailure,
    LocalDeclarationOrderingHint,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, PreparedFileReferenceTable,
};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenCursor, TokenRange};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use rustc_hash::FxHashMap;
use std::collections::HashSet;

/// Collect local declaration-ordering hints from a constant's declared type annotation.
///
/// WHY: only the declared type creates a structural ordering constraint.
/// Initializer-expression constant references are handled by
/// `constant_dependencies::add_constant_initializer_dependencies`.
pub(super) fn collect_constant_type_hints(
    declaration_syntax: &DeclarationSyntax,
    context: &mut HeaderBuildContext<'_>,
    hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
) -> Result<(), HeaderParseFailure> {
    // A `$config` contract type belongs to the provider-independent contract shell, not to the
    // ordinary declaration type graph. The resolution barrier will materialize the declaration
    // after provider binding, so retaining this as a local ordering edge would leak contract
    // vocabulary into header topology.
    if declaration_syntax.config_contract.is_some() {
        return Ok(());
    }
    let mut selection_error = None;

    for_each_named_type_in_parsed_ref(&declaration_syntax.type_annotation, &mut |type_reference| {
        if selection_error.is_none() {
            selection_error = collect_named_type_ordering_hint(
                type_reference,
                context.file_dependency_clauses,
                context.dependency_selections,
                context.source_file,
                context.string_table,
                context.path_fork,
                hints,
            )
            .err();
        }
    });
    if let Some(error) = selection_error {
        return Err(error);
    }
    collect_capacity_references_in_parsed_ref(
        &declaration_syntax.type_annotation,
        capacity_references,
    );
    Ok(())
}

/// Record one conservative local declaration-ordering hint for a named type reference.
///
/// WHAT: records the dependency spelling when a bare name matches a file dependency, otherwise
/// records the same-file spelling. Qualified names retain their complete local namespace spelling
/// for binding-time resolution through `FileVisibility`.
/// WHY: syntax preparation must not consult provider availability to decide whether a named type
/// reference is a virtual or provider dependency. Binding later resolves qualified namespace paths
/// to canonical declaration paths or drops external symbols; Stage 3 resolves retained local hints
/// into graph edges.
pub(super) fn collect_named_type_ordering_hint(
    type_reference: ParsedNamedTypeReference<'_>,
    file_dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    source_file: PathId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    hints: &mut HashSet<LocalDeclarationOrderingHint>,
) -> Result<(), HeaderParseFailure> {
    match type_reference {
        ParsedNamedTypeReference::Bare {
            name: type_name,
            span,
        } => {
            if is_reserved_builtin_symbol(string_table.resolve(type_name))
                || is_reserved_parameter_contract_symbol(string_table.resolve(type_name))
            {
                return Ok(());
            }

            let dependency_path = dependency_path_for_local_name(
                type_name,
                file_dependency_clauses,
                dependency_selections,
                string_table,
                path_fork,
            )?;
            let hint = match dependency_path {
                Some(path) => LocalDeclarationOrderingHint::provider_spelling(path),
                None => {
                    let path = path_fork
                        .try_intern_child(source_file, type_name)
                        .ok_or_else(|| {
                            HeaderParseFailure::Diagnostic(
                                token_lexical::compiler_capacity_exceeded(
                                    CapacityResource::PathNodes,
                                    span,
                                ),
                            )
                        })?;
                    LocalDeclarationOrderingHint::source_owned(path)
                }
            };
            hints.insert(hint);
        }
        ParsedNamedTypeReference::Qualified { path, span } => {
            if path.is_empty() {
                return Ok(());
            }

            // Keep every namespace component in the retained spelling. The declaration-file
            // visibility environment owns the mapping from this local path to its canonical
            // source declaration once provider binding has completed.
            let path = path_fork.try_intern_components(path).ok_or_else(|| {
                HeaderParseFailure::Diagnostic(token_lexical::compiler_capacity_exceeded(
                    CapacityResource::PathNodes,
                    span,
                ))
            })?;
            hints.insert(LocalDeclarationOrderingHint::qualified_type_spelling(path));
        }
    }
    Ok(())
}

/// Resolve one file-local dependency binding to the provider or selected symbol path it names.
///
/// WHAT: maps a visible local name to the structural path recorded by its retained clause. A
/// namespace binding resolves to the clause provider root; a direct selection resolves to the
/// provider root plus that selection's source name.
/// WHY: header syntax consumers such as type ordering and top-level const-template placement must
pub(super) fn dependency_path_for_local_name(
    local_name: StringId,
    file_dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<PathId>, HeaderParseFailure> {
    for dependency in file_dependency_clauses {
        let selections = dependency.selections(dependency_selections)?;
        if selections.is_empty() {
            if dependency.effective_namespace_local_name(string_table, path_fork)
                == Some(local_name)
            {
                return Ok(Some(dependency.dependency.path));
            }
            continue;
        }

        if let Some(selection) = selections
            .iter()
            .find(|selection| selection.local_name() == local_name)
        {
            let path = path_fork
                .try_intern_child(dependency.dependency.path, selection.source_name)
                .ok_or_else(|| {
                    HeaderParseFailure::Diagnostic(token_lexical::compiler_capacity_exceeded(
                        CapacityResource::PathNodes,
                        Some(selection.source_span),
                    ))
                })?;
            return Ok(Some(path));
        }
    }

    Ok(None)
}

// ------------------------
//  Qualified-reference ordering
// ------------------------

/// Record declaration-ordering hints for const-required expression-qualified occurrences.
///
/// WHAT: visits each prepared occurrence once and records its already-resolved target on the
/// header shell named by its module-rebased `owning_header_index`.
/// WHY: a qualified reference in a compile-time shell is a real declaration dependency even
///      though it installed no local name. Imported targets keep the provider-interface path so
///      Stage 3 classifies them through the existing provider edge arm instead of fabricating a
///      local header. A runtime function or start body never contributes, even when the same
///      header shell also owns a const-required default. Symbol collection already rebased every
///      occurrence against the aggregated header order, so a per-header occurrence scan would
///      visit the same list once per header and multiply work by header count.
pub(super) fn collect_qualified_reference_ordering_hints(
    headers: &mut [Header],
    module_symbols: &ModuleSymbols,
    binding_environment: &HeaderBindingEnvironment,
) -> Result<(), InfrastructureFailure> {
    if binding_environment.qualified_reference_targets.is_empty() {
        return Ok(());
    }

    // Occurrence lists are file-owned and map order is not observable here: insertion targets a
    // per-header hint set and this pass produces no diagnostic that could select a witness.
    for occurrences in module_symbols.qualified_references_by_source.values() {
        for occurrence in occurrences {
            if !occurrence.const_required {
                continue;
            }

            let source = occurrence.dependency.dependency_shell_id.source;
            let Some(header) = headers.get_mut(occurrence.owning_header_index) else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    "qualified-reference header index exceeded the aggregated header vector",
                ));
            };
            if header.tokens.source() != source {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    "qualified-reference header index resolved to a different source",
                ));
            }

            let Some(resolved) = binding_environment
                .qualified_reference_target(source, occurrence.dependency.path_syntax)
            else {
                continue;
            };
            let target_path = match &resolved.target {
                ResolvedQualifiedTarget::Source { declaration_path }
                    if module_symbols.constant_paths.contains(declaration_path) =>
                {
                    Some(*declaration_path)
                }
                ResolvedQualifiedTarget::Imported { local_path, .. } => Some(*local_path),
                ResolvedQualifiedTarget::Source { .. }
                | ResolvedQualifiedTarget::External { .. } => None,
            };
            if let Some(target_path) = target_path {
                header.local_ordering_hints.insert(
                    LocalDeclarationOrderingHint::qualified_reference(target_path)
                        .with_occurrence_span(resolved.selected_span),
                );
            }
        }
    }

    Ok(())
}

// ------------------------
//  Content-source shell ordering
// ------------------------

/// Record content-source ordering hints for every const-required value range in one file.
///
/// WHAT: takes the shared const-required expression classification and scans each range for path
/// tokens whose prepared row classified as a content source, recording one hint per occurrence
/// targeting that source's synthetic `content` constant.
/// WHY: a direct `.mtf` or `.md` value path reuses the synthetic content constant, so every shell
/// that folds before ordinary body emission depends on it. Runtime function and start bodies are
/// never scanned. Sharing `push_const_required_expression_ranges` keeps this ordering pass and
/// qualified-reference ordering on one definition of which defaults are compile-time content. The
/// scan stays token-level: it reads path handles and their prepared classification without
/// parsing any expression.
pub(super) fn collect_content_source_ordering_hints(
    headers: &mut [Header],
    source_tokens: &SourceTokens,
    file_references: &PreparedFileReferenceTable,
    path_syntax: &PathSyntaxTable,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), InfrastructureFailure> {
    let content_targets =
        content_source_targets(file_references, path_syntax, string_table, path_fork)?;

    let mut const_required_ranges = Vec::new();
    for (header_index, header) in headers.iter_mut().enumerate() {
        const_required_ranges.clear();
        push_const_required_expression_ranges(header, header_index, &mut const_required_ranges);
        for expression_range in &const_required_ranges {
            scan_token_range_for_content_sources(
                expression_range.range,
                source_tokens,
                &content_targets,
                &mut header.local_ordering_hints,
            )?;
        }
    }
    Ok(())
}

/// Map every content-class path row to its synthetic content constant hint target.
///
/// WHY: one lookup keyed by the authored path token's handle keeps the per-shell scan allocation
/// free; rows consumed by dependency clauses never reach the table, so a clause-consumed
/// occurrence records no content edge.
fn content_source_targets(
    file_references: &PreparedFileReferenceTable,
    path_syntax: &PathSyntaxTable,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FxHashMap<PathSyntaxId, LocalDeclarationOrderingHint>, InfrastructureFailure> {
    let mut targets = FxHashMap::default();
    for reference in file_references.iter() {
        if reference.class != PreparedFileReferenceClass::ContentSource {
            continue;
        }

        let authored_path = path_syntax
            .try_path_for_token(reference.path_syntax, reference.span)?
            .root;
        let content_path = content_constant_path(authored_path, path_fork, string_table)?;
        targets.insert(
            reference.path_syntax,
            LocalDeclarationOrderingHint::content_source(content_path, reference.path_syntax)
                .with_occurrence_span(reference.span),
        );
    }

    Ok(targets)
}

fn scan_token_range_for_content_sources(
    range: TokenRange,
    source_tokens: &SourceTokens,
    content_targets: &FxHashMap<PathSyntaxId, LocalDeclarationOrderingHint>,
    hints: &mut HashSet<LocalDeclarationOrderingHint>,
) -> Result<(), InfrastructureFailure> {
    if range.source() != source_tokens.source() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "content ordering range belongs to a different source token owner",
        ));
    }
    let cursor = source_tokens.cursor(range).map_err(|_| {
        InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "content ordering range exceeds its source token owner",
        )
    })?;
    scan_source_range_for_content_sources(cursor, content_targets, hints);
    Ok(())
}

/// Insert one content hint for every path token in the range view whose row is a content source.
///
/// WHY: bounded cursor access keeps the per-shell scan allocation-free; path handles come
/// from the shape and rows keep resolving through the existing prepared table lookup.
fn scan_source_range_for_content_sources(
    mut cursor: TokenCursor<'_>,
    content_targets: &FxHashMap<PathSyntaxId, LocalDeclarationOrderingHint>,
    hints: &mut HashSet<LocalDeclarationOrderingHint>,
) {
    while let Some(token) = cursor.advance() {
        if let Some(path_id) = token.path_syntax_id()
            && let Some(target) = content_targets.get(&path_id)
        {
            hints.insert(target.clone());
        }
        if token.is_eof() {
            break;
        }
    }
}

#[cfg(test)]
#[path = "tests/ordering_hints_tests.rs"]
mod ordering_hints_tests;

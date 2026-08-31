//! Header-stage local declaration-ordering hint collection.
//!
//! WHAT: converts type references from declaration shells into conservative local
//! declaration-ordering hints retained before provider binding.
//! WHY: syntax preparation records the dependency spelling or same-file spelling uniformly without
//! knowing which dependencies are source graph participants versus virtual or provider bindings.
//! Binding canonicalizes or drops dependency-spelled hints; Stage 3 resolves retained local hints
//! into sortable graph edges.

use crate::compiler_frontend::builtins::error_type::is_reserved_builtin_symbol;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    collect_capacity_references_in_parsed_ref, for_each_named_type_in_parsed_ref,
};
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::headers::types::{
    DependencySelection, Header, HeaderBuildContext, HeaderKind, LocalDeclarationOrderingHint,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, PreparedFileReferenceTable,
};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind};
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
) -> Result<(), CompilerError> {
    let mut selection_error = None;
    for_each_named_type_in_parsed_ref(&declaration_syntax.type_annotation, &mut |type_name| {
        if selection_error.is_none() {
            selection_error = collect_named_type_ordering_hint(
                type_name,
                context.file_dependency_clauses,
                context.dependency_selections,
                context.source_file,
                context.string_table,
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
/// WHAT: records the dependency spelling when the name matches a file dependency, otherwise records the
/// same-file spelling. Builtin symbol names are excluded as compiler-owned syntax policy.
/// WHY: syntax preparation must not consult provider availability to decide whether a named type
/// reference is a virtual or provider dependency. Binding later canonicalizes or drops dependency-spelled
/// hints using bound visibility; Stage 3 resolves retained local hints into graph edges.
pub(super) fn collect_named_type_ordering_hint(
    type_name: StringId,
    file_dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    source_file: &InternedPath,
    string_table: &mut StringTable,
    hints: &mut HashSet<LocalDeclarationOrderingHint>,
) -> Result<(), CompilerError> {
    if is_reserved_builtin_symbol(string_table.resolve(type_name)) {
        return Ok(());
    }

    // WHY: match by local name, which is either the explicit dependency alias or
    // the original symbol name from the path. This records the dependency spelling
    // when a dependency alias is used as a type reference.
    let dependency_path = dependency_path_for_local_name(
        type_name,
        file_dependency_clauses,
        dependency_selections,
        string_table,
    )?;
    let hint = match dependency_path {
        Some(path) => LocalDeclarationOrderingHint::provider_spelling(path),
        None => LocalDeclarationOrderingHint::source_owned(source_file.append(type_name)),
    };
    hints.insert(hint);
    Ok(())
}

/// Resolve one file-local dependency binding to the provider or selected symbol path it names.
///
/// WHAT: maps a visible local name to the structural path recorded by its retained clause. A
/// namespace binding resolves to the clause provider root; a direct selection resolves to the
/// provider root plus that selection's source name.
/// WHY: header syntax consumers such as type ordering and top-level const-template placement must
/// share the clause-owned selection table instead of searching a derived set of provider paths.
pub(super) fn dependency_path_for_local_name(
    local_name: StringId,
    file_dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    string_table: &mut StringTable,
) -> Result<Option<InternedPath>, CompilerError> {
    for dependency in file_dependency_clauses {
        let selections = dependency.selections(dependency_selections)?;
        if selections.is_empty() {
            if dependency.effective_namespace_local_name(string_table) == Some(local_name) {
                return Ok(Some(dependency.dependency.path.clone()));
            }
            continue;
        }

        if let Some(selection) = selections
            .iter()
            .find(|selection| selection.local_name() == local_name)
        {
            return Ok(Some(
                dependency.dependency.path.append(selection.source_name),
            ));
        }
    }

    Ok(None)
}

// ------------------------
//  Content-source shell ordering
// ------------------------

/// Record content-source ordering hints for every pre-body declaration shell in one file.
///
/// WHAT: scans each declaration shell's value-token range for path tokens whose prepared row
/// classified as a content source, and records one hint per occurrence targeting that source's
/// synthetic `content` constant. Runtime function and start bodies are never scanned.
/// WHY: a direct `.mtf` or `.md` value path reuses the synthetic content constant, so every shell
/// that folds before ordinary body emission depends on it. The scan stays token-level: it reads
/// path handles and their prepared classification without parsing any expression.
pub(super) fn collect_content_source_ordering_hints(
    headers: &mut [Header],
    file_references: &PreparedFileReferenceTable,
    path_syntax: &PathSyntaxTable,
    string_table: &mut StringTable,
) -> Result<(), CompilerError> {
    let content_targets = content_source_targets(file_references, path_syntax, string_table)?;

    for header in headers {
        let Header {
            kind,
            tokens,
            local_ordering_hints,
            ..
        } = header;

        match kind {
            HeaderKind::Constant { declaration } => {
                scan_tokens_for_content_sources(
                    &declaration.initializer_tokens,
                    &content_targets,
                    local_ordering_hints,
                );
            }

            // Const-template tokens cover both the template head and body, including the
            // top-level compile-time fragments folded before body emission.
            HeaderKind::ConstTemplate { .. } => {
                scan_tokens_for_content_sources(
                    &tokens.tokens,
                    &content_targets,
                    local_ordering_hints,
                );
            }

            HeaderKind::Function { signature, .. } => {
                for parameter in &signature.parameters {
                    scan_tokens_for_content_sources(
                        &parameter.default_tokens,
                        &content_targets,
                        local_ordering_hints,
                    );
                }
            }

            HeaderKind::Struct { fields, .. } => {
                for field in fields {
                    scan_tokens_for_content_sources(
                        &field.default_tokens,
                        &content_targets,
                        local_ordering_hints,
                    );
                }
            }

            HeaderKind::Choice { variants, .. } => {
                for variant in variants {
                    let ChoiceVariantPayloadSyntax::Record { fields } = &variant.payload else {
                        continue;
                    };
                    for field in fields {
                        scan_tokens_for_content_sources(
                            &field.default_tokens,
                            &content_targets,
                            local_ordering_hints,
                        );
                    }
                }
            }

            // Runtime bodies fold after the synthetic content constants are complete, and the
            // remaining shells hold no pre-body value expressions.
            _ => {}
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
) -> Result<FxHashMap<PathSyntaxId, LocalDeclarationOrderingHint>, CompilerError> {
    let mut targets = FxHashMap::default();
    for reference in file_references.iter() {
        if reference.class != PreparedFileReferenceClass::ContentSource {
            continue;
        }

        let authored_path = &path_syntax
            .try_path_for_token(reference.path_syntax, &reference.location)?
            .root;
        targets.insert(
            reference.path_syntax,
            LocalDeclarationOrderingHint::content_source(
                content_constant_path(authored_path, string_table),
                reference.path_syntax,
            ),
        );
    }

    Ok(targets)
}

/// Insert one content hint for every path token in the slice whose row is a content source.
fn scan_tokens_for_content_sources(
    tokens: &[Token],
    content_targets: &FxHashMap<PathSyntaxId, LocalDeclarationOrderingHint>,
    hints: &mut HashSet<LocalDeclarationOrderingHint>,
) {
    for token in tokens {
        let TokenKind::Path(path_id) = token.kind else {
            continue;
        };

        if let Some(target) = content_targets.get(&path_id) {
            hints.insert(target.clone());
        }
    }
}

#[cfg(test)]
#[path = "tests/ordering_hints_tests.rs"]
mod ordering_hints_tests;

//! Header local declaration-ordering hint canonicalization.
//!
//! WHAT: rewrites dependency-spelled local declaration-ordering hints into canonical resolved
//! symbol paths using bound visibility, and drops external or binding-only dependency hints.
//! WHY: Stage 3 compares exact header graph keys, so retained hints must use the same canonical
//! paths that dependency preparation exposes through file visibility. Same-file hints are preserved.

use crate::compiler_frontend::compiler_errors::compiler_error_to_diagnostic;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::declaration_syntax::type_syntax::ParsedNamedTypeReference;
use crate::compiler_frontend::headers::binding_environment::{
    FileVisibility, HeaderBindingEnvironment, NamespaceMemberLookup, NamespaceTypeMember,
    lookup_namespace_member,
};
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::types::{
    DependencySelection, Header, LocalDeclarationOrderingHint, LocalDeclarationOrderingHintOrigin,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

use std::collections::HashSet;

/// Result of resolving one parsed named type through a declaration file's visibility package.
///
/// WHAT: distinguishes a canonical source declaration from an external type and an unresolved
/// spelling without reducing a qualified path to its terminal component.
/// WHY: Stage 3 ordering and alias waiting use the same namespace-aware identity route; external
/// types have no header graph node, while unresolved names remain available for later diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VisibleNamedTypeResolution {
    Declaration(InternedPath),
    External,
    Unresolved,
}

/// Resolve a bare or namespace-qualified parsed type through file visibility.
///
/// WHAT: bare references use the visible alias/source maps; qualified references walk namespace
/// records and resolve their final type member to its canonical declaration path.
/// WHY: namespace aliases and direct declarations can share terminal names, so comparing only the
/// final textual component loses the declaration identity needed by ordering and waiting passes.
pub(crate) fn resolve_visible_named_type_path(
    type_reference: ParsedNamedTypeReference<'_>,
    visibility: &FileVisibility,
) -> VisibleNamedTypeResolution {
    match type_reference {
        ParsedNamedTypeReference::Bare(name) => {
            if let Some(target) = visibility
                .visible_type_alias_names
                .get(&name)
                .or_else(|| visibility.visible_source_names.get(&name))
            {
                return VisibleNamedTypeResolution::Declaration(target.local_path().clone());
            }

            if visibility.visible_external_symbols.contains_key(&name) {
                VisibleNamedTypeResolution::External
            } else {
                VisibleNamedTypeResolution::Unresolved
            }
        }
        ParsedNamedTypeReference::Qualified(path) => {
            let Some((&root, members)) = path.split_first() else {
                return VisibleNamedTypeResolution::Unresolved;
            };
            let Some(mut record) = visibility.visible_namespace_records.get(&root) else {
                return VisibleNamedTypeResolution::Unresolved;
            };
            let Some((&final_name, intermediate)) = members.split_last() else {
                return VisibleNamedTypeResolution::Unresolved;
            };

            for segment in intermediate {
                match lookup_namespace_member(record, *segment) {
                    NamespaceMemberLookup::ChildNamespace(child) => record = child,
                    NamespaceMemberLookup::Value(_)
                    | NamespaceMemberLookup::Type
                    | NamespaceMemberLookup::Missing => {
                        return VisibleNamedTypeResolution::Unresolved;
                    }
                }
            }

            match record.type_members.get(&final_name) {
                Some(NamespaceTypeMember::SourceDeclaration(target)) => {
                    VisibleNamedTypeResolution::Declaration(target.local_path().clone())
                }
                Some(NamespaceTypeMember::ExternalSymbol(_)) => {
                    VisibleNamedTypeResolution::External
                }
                None => VisibleNamedTypeResolution::Unresolved,
            }
        }
    }
}

/// Canonicalize retained local declaration-ordering hints using bound visibility.
///
/// WHAT: for each hint, if its path matches a file dependency the dependency's local name is
/// resolved through bound visibility to a canonical source path. Qualified namespace spellings
/// are resolved through the declaration file's namespace records. External or virtual/provider
/// dependencies with no header graph participant are dropped; same-file or unresolved hints are
/// preserved.
pub(super) fn canonicalize_local_ordering_hints(
    headers: &mut [Header],
    binding_environment: &HeaderBindingEnvironment,
    file_dependency_clauses_by_source: &FxHashMap<InternedPath, Vec<RetainedDependencyClause>>,
    dependency_selections_by_source: &FxHashMap<InternedPath, Vec<DependencySelection>>,
    string_table: &mut StringTable,
) -> Result<(), DiagnosticBag> {
    let mut diagnostic_bag = DiagnosticBag::new();

    for header in headers.iter_mut() {
        let visibility = match binding_environment.visibility_for(&header.source_file) {
            Ok(visibility) => visibility,
            Err(error) => {
                diagnostic_bag.push(compiler_error_to_diagnostic(&error));
                continue;
            }
        };

        let file_dependency_clauses = file_dependency_clauses_by_source
            .get(&header.source_file)
            .map(|dependencies| dependencies.as_slice())
            .unwrap_or(&[]);

        let mut canonical: HashSet<LocalDeclarationOrderingHint> =
            HashSet::with_capacity(header.local_ordering_hints.len());

        for hint in header.local_ordering_hints.drain() {
            let selection_table = dependency_selections_by_source
                .get(&header.source_file)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if hint.origin() == LocalDeclarationOrderingHintOrigin::QualifiedTypeSpelling {
                match resolve_visible_named_type_path(
                    ParsedNamedTypeReference::Qualified(hint.path().as_components()),
                    visibility,
                ) {
                    VisibleNamedTypeResolution::Declaration(path) => {
                        canonical.insert(LocalDeclarationOrderingHint::source_owned(path));
                    }
                    VisibleNamedTypeResolution::External
                    | VisibleNamedTypeResolution::Unresolved => {
                        // Non-declarations have no header graph participant. AST resolution owns
                        // the eventual unknown-type or namespace-misuse diagnostic.
                    }
                }
                continue;
            }

            let mut matching_dependency = None;
            for dependency in file_dependency_clauses {
                let selections = match dependency.selections(selection_table) {
                    Ok(selections) => selections,
                    Err(error) => {
                        diagnostic_bag.push(compiler_error_to_diagnostic(&error));
                        break;
                    }
                };
                if dependency.dependency.path == *hint.path() && selections.is_empty() {
                    matching_dependency = dependency
                        .effective_namespace_local_name(string_table)
                        .map(|local_name| (dependency, local_name));
                    if matching_dependency.is_some() {
                        break;
                    }
                    continue;
                }

                if let Some(selection) = selections.iter().find(|selection| {
                    dependency.dependency.path.append(selection.source_name) == *hint.path()
                }) {
                    matching_dependency = Some((dependency, selection.local_name()));
                    break;
                }
            }
            if let Some((_dependency, local_name)) = matching_dependency {
                if let Some(resolved_path) = visibility
                    .visible_source_names
                    .get(&local_name)
                    .or_else(|| visibility.visible_type_alias_names.get(&local_name))
                {
                    canonical.insert(LocalDeclarationOrderingHint::source_owned(
                        resolved_path.local_path().clone(),
                    ));
                }
                // External symbols and virtual or provider dependencies have no header graph
                // participant, so the dependency-spelled hint is dropped here.
            } else {
                // Same-file or already-canonical hint: preserve it.
                canonical.insert(hint);
            }
        }

        header.local_ordering_hints = canonical;
    }

    if diagnostic_bag.has_errors() {
        Err(diagnostic_bag)
    } else {
        Ok(())
    }
}

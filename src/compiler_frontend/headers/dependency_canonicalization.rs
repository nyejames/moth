//! Header local declaration-ordering hint canonicalization.
//!
//! WHAT: rewrites dependency-spelled local declaration-ordering hints into canonical resolved
//! symbol paths using bound visibility, and drops external or binding-only dependency hints.
//! WHY: Stage 3 compares exact header graph keys, so retained hints must use the same canonical
//! paths that dependency preparation exposes through file visibility. Same-file hints are preserved.

use crate::compiler_frontend::compiler_errors::compiler_error_to_diagnostic;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::types::{
    DependencySelection, Header, LocalDeclarationOrderingHint,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

use std::collections::HashSet;

/// Canonicalize retained local declaration-ordering hints using bound visibility.
///
/// WHAT: for each hint, if its path matches a file dependency the dependency's local name is resolved
/// through bound visibility to a canonical source path; external or virtual/provider dependencies
/// with no header graph participant are dropped. Same-file or already-canonical hints are
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

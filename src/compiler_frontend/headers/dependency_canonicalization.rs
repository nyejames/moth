//! Header local declaration-ordering hint canonicalization.
//!
//! WHAT: rewrites import-spelled local declaration-ordering hints into canonical resolved
//! symbol paths using bound visibility, and drops external or binding-only import hints.
//! WHY: Stage 3 compares exact header graph keys, so retained hints must use the same canonical
//! paths that import preparation exposes through file visibility. Same-file hints are preserved.

use crate::compiler_frontend::compiler_errors::compiler_error_to_diagnostic;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::headers::import_environment::HeaderImportEnvironment;
use crate::compiler_frontend::headers::parse_file_headers::FileImport;
use crate::compiler_frontend::headers::types::{Header, LocalDeclarationOrderingHint};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use rustc_hash::FxHashMap;

use std::collections::HashSet;

/// Canonicalize retained local declaration-ordering hints using bound visibility.
///
/// WHAT: for each hint, if its path matches a file import the import's local name is resolved
/// through bound visibility to a canonical source path; external or virtual/provider imports
/// with no header graph participant are dropped. Same-file or already-canonical hints are
/// preserved.
pub(super) fn canonicalize_local_ordering_hints(
    headers: &mut [Header],
    import_environment: &HeaderImportEnvironment,
    file_imports_by_source: &FxHashMap<InternedPath, Vec<FileImport>>,
) -> Result<(), DiagnosticBag> {
    let mut diagnostic_bag = DiagnosticBag::new();

    for header in headers.iter_mut() {
        let visibility = match import_environment.visibility_for(&header.source_file) {
            Ok(visibility) => visibility,
            Err(error) => {
                diagnostic_bag.push(compiler_error_to_diagnostic(&error));
                continue;
            }
        };

        let file_imports = file_imports_by_source
            .get(&header.source_file)
            .map(|imports| imports.as_slice())
            .unwrap_or(&[]);

        let mut canonical: HashSet<LocalDeclarationOrderingHint> =
            HashSet::with_capacity(header.local_ordering_hints.len());

        for hint in header.local_ordering_hints.drain() {
            let matching_import = file_imports
                .iter()
                .find(|import| import.provider.path == hint.path);

            if let Some(import) = matching_import {
                let local_name = match import.alias {
                    Some(alias) => alias,
                    None => match import.provider.path.name() {
                        Some(name) => name,
                        None => {
                            canonical.insert(hint);
                            continue;
                        }
                    },
                };

                if let Some(resolved_path) = visibility
                    .visible_source_names
                    .get(&local_name)
                    .or_else(|| visibility.visible_type_alias_names.get(&local_name))
                {
                    canonical.insert(LocalDeclarationOrderingHint::new(
                        resolved_path.local_path().clone(),
                    ));
                }
                // External symbols and virtual or provider imports have no header graph
                // participant, so the import-spelled hint is dropped here.
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

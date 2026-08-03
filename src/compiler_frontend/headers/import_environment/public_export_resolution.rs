//! Header-stage public export boundary resolution.
//!
//! WHAT: resolves cross-package and cross-module-root imports through the target module's public
//! export maps.
//! WHY: source-backed package modules and regular module roots expose symbols only through their
//! prepared root files; external importers cannot bypass those public surfaces.
//! MUST NOT: perform general import target resolution (that belongs in `target_resolution.rs`).

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ImportPublicSurfaceType};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::import_environment::diagnostics;
use crate::compiler_frontend::headers::import_environment::target_resolution::suffix_matches_with_optional_source_extension;
use crate::compiler_frontend::headers::module_symbols::{
    ModuleRootBoundary, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::{FxHashMap, FxHashSet};

/// Boxed diagnostic result for public export boundary checks.
///
/// WHAT: gives source-backed package and module privacy checks one small error boundary.
/// WHY: both callers already propagate boxed diagnostics, so the checks can preserve
///      structured errors without unboxing and reboxing them.
type BoundaryCheckResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// Result of looking up an import path against a module public export.
pub(crate) enum PublicExportLookupResult {
    /// This import does not target a public export; use normal file-based resolution.
    NotAPublicExportBoundary,
    /// The public export surface exports a source symbol with this canonical path.
    ExportedSource {
        path: InternedPath,
        exported_entries: FxHashSet<PublicExportEntry>,
    },
    /// The public export surface exports an external package symbol.
    ExportedExternal { symbol_id: ExternalSymbolId },
    /// The import targets a public export surface but the requested symbol is not exported.
    NotExported {
        public_surface_name: String,
        public_surface_type: PublicExportSurfaceType,
    },
}

/// Classification of public export for diagnostic messages.
pub(crate) enum PublicExportSurfaceType {
    SourcePackage,
    ModuleRoot,
}

/// Input bundle for public export boundary resolution.
///
/// WHY: boundary resolution needs both the import path and the module's public export metadata.
pub(crate) struct PublicExportResolutionInput<'a> {
    pub(crate) importer_file: &'a InternedPath,
    pub(crate) header_path: &'a InternedPath,
    pub(crate) source_package_public_exports: &'a FxHashMap<String, FxHashSet<PublicExportEntry>>,
    pub(crate) file_package_membership: &'a FxHashMap<InternedPath, String>,
    pub(crate) module_root_public_exports:
        &'a FxHashMap<InternedPath, FxHashSet<PublicExportEntry>>,
    pub(crate) file_module_membership: &'a FxHashMap<InternedPath, InternedPath>,
    pub(crate) module_root_boundaries: &'a [ModuleRootBoundary],
    pub(crate) string_table: &'a StringTable,
}

/// Attempt to resolve an import through a source-backed package or module-root public surface.
///
/// WHAT: checks whether the import path starts with a known package prefix or module root,
/// and whether the importer is outside that module. If so, looks up the symbol name in the
/// public surface's exported entries.
pub(crate) fn resolve_public_export_boundary(
    input: &PublicExportResolutionInput<'_>,
) -> Option<PublicExportLookupResult> {
    // Try cross-package public export resolution first.
    if let Some(result) = try_resolve_package_public_export(input) {
        return Some(result);
    }

    // Try cross-module-root public export resolution.
    try_resolve_module_root_public_export(input)
}

/// Cross-package public export lookup.
///
/// WHAT: when an import path starts with a package prefix and the importer is outside that
/// package, the symbol must be exported by the module's root-file public surface.
fn try_resolve_package_public_export(
    input: &PublicExportResolutionInput<'_>,
) -> Option<PublicExportLookupResult> {
    let components = input.header_path.as_components();
    if components.is_empty() {
        return None;
    }

    let first = input.string_table.resolve(components[0]);
    let package_prefix = input
        .source_package_public_exports
        .keys()
        .find(|p| *p == first)?;

    // Internal imports within the same package use normal file-based resolution.
    let importer_package = input.file_package_membership.get(input.importer_file);
    if importer_package.map(|s| s.as_str()) == Some(package_prefix) {
        return Some(PublicExportLookupResult::NotAPublicExportBoundary);
    }

    // Imports from outside the source-backed package must request exactly one public symbol from the
    // public root. Extra path components are implementation details, not part of the public API.
    if components.len() != 2 {
        return Some(PublicExportLookupResult::NotExported {
            public_surface_name: package_prefix.clone(),
            public_surface_type: PublicExportSurfaceType::SourcePackage,
        });
    }

    let symbol_name = components[1];
    let exports = input.source_package_public_exports.get(package_prefix)?;
    for entry in exports {
        if entry.export_name == symbol_name {
            match &entry.target {
                PublicExportTarget::Source { path, .. } => {
                    return Some(PublicExportLookupResult::ExportedSource {
                        path: path.clone(),
                        exported_entries: exports.clone(),
                    });
                }
                PublicExportTarget::External(symbol_id) => {
                    return Some(PublicExportLookupResult::ExportedExternal {
                        symbol_id: *symbol_id,
                    });
                }
            }
        }
    }

    Some(PublicExportLookupResult::NotExported {
        public_surface_name: package_prefix.clone(),
        public_surface_type: PublicExportSurfaceType::SourcePackage,
    })
}

/// Derive the canonical module-boundary path for an import authored from `importer_file`.
///
/// WHAT: prepends the importer module root's public import prefix (relative to the entry root)
/// to the authored import path components. For an entry-root importer the prefix is empty, so
/// ordinary `@child` stays unchanged. For a nested module importer, `@child` becomes
/// `<importer-prefix>/child`, which is the canonical module-root-relative interpretation.
/// WHY: grouped, direct and namespace imports share one boundary classification algorithm.
///      Centralising the derivation prevents the physical-parent or `@./` interpretation from
///      reappearing in a sibling owner.
pub(super) fn effective_module_boundary_path(
    importer_file: &InternedPath,
    import_path: &InternedPath,
    file_module_membership: &FxHashMap<InternedPath, InternedPath>,
    module_root_boundaries: &[ModuleRootBoundary],
) -> InternedPath {
    let module_root_prefix: &[StringId] = file_module_membership
        .get(importer_file)
        .and_then(|root| {
            module_root_boundaries
                .iter()
                .find(|boundary| &boundary.module_root == root)
                .map(|boundary| boundary.import_prefix.as_components())
        })
        .unwrap_or(&[]);

    let mut combined = module_root_prefix.to_vec();
    combined.extend_from_slice(import_path.as_components());
    InternedPath::from_components(combined)
}

/// Cross-module-root public export lookup.
///
/// WHAT: when an import path targets a regular module root under the entry root and the
/// importer is outside that module, the symbol must be exported by the module root file.
fn try_resolve_module_root_public_export(
    input: &PublicExportResolutionInput<'_>,
) -> Option<PublicExportLookupResult> {
    if input.module_root_boundaries.is_empty() {
        return None;
    }

    let components = input.header_path.as_components();
    if components.is_empty() {
        return None;
    }

    let effective_path = effective_module_boundary_path(
        input.importer_file,
        input.header_path,
        input.file_module_membership,
        input.module_root_boundaries,
    );

    // Find the longest matching module root prefix.
    for boundary in input.module_root_boundaries {
        if effective_path.starts_with(&boundary.import_prefix) {
            // Internal imports within the same module root use normal resolution.
            let importer_root = input.file_module_membership.get(input.importer_file);
            if importer_root == Some(&boundary.module_root) {
                return Some(PublicExportLookupResult::NotAPublicExportBoundary);
            }

            // Named module roots use the root path as their public API prefix. The entry root
            // has no prefix, so its public re-exports stay addressable at their real
            // source paths, but arbitrary paths with the same final name must not match.
            let prefix_len = boundary.import_prefix.as_components().len();
            let effective_components = effective_path.as_components();
            let public_suffix = &effective_components[prefix_len..];
            let exports = input
                .module_root_public_exports
                .get(&boundary.module_root)?;

            if prefix_len == 0 {
                for entry in exports {
                    if let PublicExportTarget::Source { path, .. } = &entry.target
                        && suffix_matches_with_optional_source_extension(
                            path,
                            &effective_path,
                            input.string_table,
                        )
                    {
                        return Some(PublicExportLookupResult::ExportedSource {
                            path: path.clone(),
                            exported_entries: exports.clone(),
                        });
                    }
                }

                return Some(PublicExportLookupResult::NotExported {
                    public_surface_name: boundary
                        .import_prefix
                        .to_portable_string(input.string_table),
                    public_surface_type: PublicExportSurfaceType::ModuleRoot,
                });
            }

            let symbol_name = if public_suffix.len() == 1 {
                Some(public_suffix[0])
            } else {
                None
            };
            let Some(symbol_name) = symbol_name else {
                return Some(PublicExportLookupResult::NotExported {
                    public_surface_name: boundary
                        .import_prefix
                        .to_portable_string(input.string_table),
                    public_surface_type: PublicExportSurfaceType::ModuleRoot,
                });
            };

            for entry in exports {
                if entry.export_name == symbol_name {
                    match &entry.target {
                        PublicExportTarget::Source { path, .. } => {
                            return Some(PublicExportLookupResult::ExportedSource {
                                path: path.clone(),
                                exported_entries: exports.clone(),
                            });
                        }
                        PublicExportTarget::External(symbol_id) => {
                            return Some(PublicExportLookupResult::ExportedExternal {
                                symbol_id: *symbol_id,
                            });
                        }
                    }
                }
            }
            return Some(PublicExportLookupResult::NotExported {
                public_surface_name: boundary
                    .import_prefix
                    .to_portable_string(input.string_table),
                public_surface_type: PublicExportSurfaceType::ModuleRoot,
            });
        }
    }

    None
}

/// Input bundle for source-backed package boundary checking.
pub(crate) struct SourcePackageBoundaryCheckInput<'a> {
    pub(crate) importer_file: &'a InternedPath,
    pub(crate) target_file: &'a InternedPath,
    pub(crate) requested_path: &'a InternedPath,
    pub(crate) location: SourceLocation,
    pub(crate) file_package_membership: &'a FxHashMap<InternedPath, String>,
    pub(crate) source_package_root_files: &'a FxHashMap<String, InternedPath>,
    pub(crate) string_table: &'a mut StringTable,
}

/// Enforces source-backed package public-surface privacy for concrete source-file imports.
///
/// WHAT: after normal source resolution reaches a file inside a source-backed package, an importer
/// outside that package may only import the package's prepared root file. Grouped public symbol
/// imports should already have resolved through `resolve_public_export_boundary`.
pub(crate) fn check_source_package_boundary(
    input: SourcePackageBoundaryCheckInput<'_>,
) -> BoundaryCheckResult<()> {
    let Some(target_package) = input.file_package_membership.get(input.target_file) else {
        return Ok(());
    };

    let importer_package = input.file_package_membership.get(input.importer_file);
    if importer_package.map(String::as_str) == Some(target_package.as_str()) {
        return Ok(());
    }

    if input
        .source_package_root_files
        .get(target_package)
        .is_some_and(|root_file| input.target_file == root_file)
    {
        return Ok(());
    }

    let public_surface_name_id = input.string_table.intern(target_package);
    Err(Box::new(
        CompilerDiagnostic::not_exported_by_public_surface(
            input.requested_path.clone(),
            public_surface_name_id,
            ImportPublicSurfaceType::SourcePackage,
            input.location,
        ),
    ))
}

/// Input bundle for module boundary checking.
///
/// WHY: cross-module-root imports must respect the target module's public root even when they
/// resolved through normal file-based path matching.
pub(crate) struct ModuleBoundaryCheckInput<'a> {
    pub(crate) importer_file: &'a InternedPath,
    pub(crate) target_file: &'a InternedPath,
    pub(crate) symbol_path: &'a InternedPath,
    pub(crate) location: SourceLocation,
    pub(crate) file_module_membership: &'a FxHashMap<InternedPath, InternedPath>,
    pub(crate) module_root_public_exports:
        &'a FxHashMap<InternedPath, FxHashSet<PublicExportEntry>>,
}

/// Enforces module-private boundaries for cross-module-root imports.
///
/// WHAT: after an import resolves to a concrete source file, if the importer and target are in
/// different module roots, the symbol must be exported by the target module's public root.
/// WHY: ordinary source declarations are importable inside one module, but cross-module imports
/// must use the target module's explicit public export surface.
pub(crate) fn check_module_boundary(
    input: ModuleBoundaryCheckInput<'_>,
) -> BoundaryCheckResult<()> {
    let importer_root = input.file_module_membership.get(input.importer_file);
    let target_root = input.file_module_membership.get(input.target_file);

    // Skip if either file has no module root membership (e.g., source-backed packages handled separately).
    let (Some(importer_root), Some(target_root)) = (importer_root, target_root) else {
        return Ok(());
    };

    // Same module root: no boundary.
    if importer_root == target_root {
        return Ok(());
    }

    // Different module roots: grouped public imports should already have resolved through the
    // target public surface. Direct source-path resolution here is therefore a boundary violation.
    if input.module_root_public_exports.contains_key(target_root) {
        return Err(Box::new(diagnostics::cross_module_import_not_exported(
            input.symbol_path,
            input.location,
        )));
    }

    // Target module has no public root surface.
    Err(Box::new(diagnostics::missing_module_root_public_surface(
        input.symbol_path,
        input.location,
    )))
}

#[cfg(test)]
#[path = "tests/public_export_resolution_tests.rs"]
mod tests;

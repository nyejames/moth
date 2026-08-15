//! Header-stage public export boundary resolution.
//!
//! WHAT: resolves cross-package and cross-module-root dependencies through the target module's public
//! export maps.
//! WHY: source-backed package modules and regular module roots expose symbols only through their
//! prepared root files; external consumers cannot bypass those public surfaces.
//! MUST NOT: perform general dependency target resolution (that belongs in `target_resolution.rs`).

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ImportPublicSurfaceType};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::diagnostics;
use crate::compiler_frontend::headers::binding_environment::target_resolution::suffix_matches_with_optional_source_extension;
use crate::compiler_frontend::headers::module_symbols::{
    ModuleRootBoundary, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::symbols::identity::DependencySelectionId;
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

/// Result of looking up a dependency path against a module public export.
pub(crate) enum PublicExportLookupResult {
    /// This dependency does not target a public export; use normal file-based resolution.
    NotAPublicExportBoundary,
    /// The public export surface exports a source symbol with this canonical path.
    ExportedSource {
        path: InternedPath,
        exported_entries: FxHashSet<PublicExportEntry>,
    },
    /// The public surface exports one provider-backed selection. Its shell and selected provider
    /// name must be joined against the completed provider interface by the binding consumer.
    ExportedProviderSelection {
        selection: DependencySelectionId,
        source_name: StringId,
        diagnostic_path: InternedPath,
    },
    /// The public export surface exports an external package symbol.
    ExportedExternal { symbol_id: ExternalSymbolId },
    /// The dependency targets a public export surface but the requested symbol is not exported.
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
/// WHY: boundary resolution needs both the dependency path and the module's public export metadata.
pub(crate) struct PublicExportResolutionInput<'a> {
    pub(crate) consumer_file: &'a InternedPath,
    pub(crate) header_path: &'a InternedPath,
    pub(crate) source_package_public_exports: &'a FxHashMap<String, FxHashSet<PublicExportEntry>>,
    pub(crate) file_package_membership: &'a FxHashMap<InternedPath, String>,
    pub(crate) module_root_public_exports:
        &'a FxHashMap<InternedPath, FxHashSet<PublicExportEntry>>,
    pub(crate) file_module_membership: &'a FxHashMap<InternedPath, InternedPath>,
    pub(crate) module_root_boundaries: &'a [ModuleRootBoundary],
    pub(crate) string_table: &'a StringTable,
}

/// Attempt to resolve a dependency through a source-backed package or module-root public surface.
///
/// WHAT: checks whether the dependency path starts with a known package prefix or module root,
/// and whether the consumer is outside that module. If so, looks up the symbol name in the
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
/// WHAT: when a dependency path starts with a package prefix and the consumer is outside that
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

    // Internal dependencies within the same package use normal file-based resolution.
    let consumer_package = input.file_package_membership.get(input.consumer_file);
    if consumer_package.map(|s| s.as_str()) == Some(package_prefix) {
        return Some(PublicExportLookupResult::NotAPublicExportBoundary);
    }

    // Dependencies from outside the source-backed package must request exactly one public symbol from the
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
                PublicExportTarget::SourceDeclaration { path } => {
                    return Some(PublicExportLookupResult::ExportedSource {
                        path: path.clone(),
                        exported_entries: exports.clone(),
                    });
                }
                PublicExportTarget::ProviderSelection {
                    selection,
                    source_name,
                    diagnostic_path,
                } => {
                    return Some(PublicExportLookupResult::ExportedProviderSelection {
                        selection: *selection,
                        source_name: *source_name,
                        diagnostic_path: diagnostic_path.clone(),
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

/// Derive the canonical module-boundary path for a dependency authored from `consumer_file`.
///
/// WHAT: prepends the consumer module root's public dependency prefix (relative to the entry root)
/// to the authored dependency path components. For an entry-root consumer the prefix is empty, so
/// ordinary `@child` stays unchanged. For a nested module consumer, `@child` becomes
/// `<consumer-prefix>/child`, which is the canonical module-root-relative interpretation.
/// WHY: direct-selection and namespace dependencies share one boundary classification algorithm.
///      Centralising the derivation prevents the physical-parent or `@./` interpretation from
///      reappearing in a sibling owner.
pub(super) fn effective_module_boundary_path(
    consumer_file: &InternedPath,
    dependency_path: &InternedPath,
    file_module_membership: &FxHashMap<InternedPath, InternedPath>,
    module_root_boundaries: &[ModuleRootBoundary],
) -> InternedPath {
    let module_root_prefix: &[StringId] = file_module_membership
        .get(consumer_file)
        .and_then(|root| {
            module_root_boundaries
                .iter()
                .find(|boundary| &boundary.module_root == root)
                .map(|boundary| boundary.dependency_prefix.as_components())
        })
        .unwrap_or(&[]);

    let mut combined = module_root_prefix.to_vec();
    combined.extend_from_slice(dependency_path.as_components());
    InternedPath::from_components(combined)
}

/// Cross-module-root public export lookup.
///
/// WHAT: when a dependency path targets a regular module root under the entry root and the
/// consumer is outside that module, the symbol must be exported by the module root file.
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
        input.consumer_file,
        input.header_path,
        input.file_module_membership,
        input.module_root_boundaries,
    );

    // Find the longest matching module root prefix.
    for boundary in input.module_root_boundaries {
        if effective_path.starts_with(&boundary.dependency_prefix) {
            // Internal dependencies within the same module root use normal resolution.
            let consumer_root = input.file_module_membership.get(input.consumer_file);
            if consumer_root == Some(&boundary.module_root) {
                return Some(PublicExportLookupResult::NotAPublicExportBoundary);
            }

            // Named module roots use the root path as their public API prefix. The entry root
            // has no prefix, so its public re-exports stay addressable at their real
            // source paths, but arbitrary paths with the same final name must not match.
            let prefix_len = boundary.dependency_prefix.as_components().len();
            let effective_components = effective_path.as_components();
            let public_suffix = &effective_components[prefix_len..];
            let exports = input
                .module_root_public_exports
                .get(&boundary.module_root)?;

            if prefix_len == 0 {
                // The entry root has no module prefix. Source declarations retain their
                // canonical path matching below; provider selections have no source path by
                // design, so their public API name is the only semantic lookup key available.
                if effective_components.len() == 1 {
                    let public_name = effective_components[0];
                    for entry in exports {
                        if entry.export_name != public_name {
                            continue;
                        }

                        match &entry.target {
                            PublicExportTarget::ProviderSelection {
                                selection,
                                source_name,
                                diagnostic_path,
                            } => {
                                return Some(PublicExportLookupResult::ExportedProviderSelection {
                                    selection: *selection,
                                    source_name: *source_name,
                                    diagnostic_path: diagnostic_path.clone(),
                                });
                            }
                            PublicExportTarget::External(symbol_id) => {
                                return Some(PublicExportLookupResult::ExportedExternal {
                                    symbol_id: *symbol_id,
                                });
                            }
                            PublicExportTarget::SourceDeclaration { .. } => {}
                        }
                    }
                }

                for entry in exports {
                    if let Some(path) = entry.target.source_path()
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
                        .dependency_prefix
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
                        .dependency_prefix
                        .to_portable_string(input.string_table),
                    public_surface_type: PublicExportSurfaceType::ModuleRoot,
                });
            };

            for entry in exports {
                if entry.export_name == symbol_name {
                    match &entry.target {
                        PublicExportTarget::SourceDeclaration { path } => {
                            return Some(PublicExportLookupResult::ExportedSource {
                                path: path.clone(),
                                exported_entries: exports.clone(),
                            });
                        }
                        PublicExportTarget::ProviderSelection {
                            selection,
                            source_name,
                            diagnostic_path,
                        } => {
                            return Some(PublicExportLookupResult::ExportedProviderSelection {
                                selection: *selection,
                                source_name: *source_name,
                                diagnostic_path: diagnostic_path.clone(),
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
                    .dependency_prefix
                    .to_portable_string(input.string_table),
                public_surface_type: PublicExportSurfaceType::ModuleRoot,
            });
        }
    }

    None
}

/// Input bundle for source-backed package boundary checking.
pub(crate) struct SourcePackageBoundaryCheckInput<'a> {
    pub(crate) consumer_file: &'a InternedPath,
    pub(crate) target_file: &'a InternedPath,
    pub(crate) requested_path: &'a InternedPath,
    pub(crate) location: SourceLocation,
    pub(crate) file_package_membership: &'a FxHashMap<InternedPath, String>,
    pub(crate) source_package_root_files: &'a FxHashMap<String, InternedPath>,
    pub(crate) string_table: &'a mut StringTable,
}

/// Enforces source-backed package public-surface privacy for concrete source-file dependencies.
///
/// WHAT: after normal source resolution reaches a file inside a source-backed package, an consumer
/// outside that package may only dependency the package's prepared root file. Grouped public symbol
/// dependencies should already have resolved through `resolve_public_export_boundary`.
pub(crate) fn check_source_package_boundary(
    input: SourcePackageBoundaryCheckInput<'_>,
) -> BoundaryCheckResult<()> {
    let Some(target_package) = input.file_package_membership.get(input.target_file) else {
        return Ok(());
    };

    let consumer_package = input.file_package_membership.get(input.consumer_file);
    if consumer_package.map(String::as_str) == Some(target_package.as_str()) {
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
/// WHY: cross-module-root dependencies must respect the target module's public root even when they
/// resolved through normal file-based path matching.
pub(crate) struct ModuleBoundaryCheckInput<'a> {
    pub(crate) consumer_file: &'a InternedPath,
    pub(crate) target_file: &'a InternedPath,
    pub(crate) symbol_path: &'a InternedPath,
    pub(crate) location: SourceLocation,
    pub(crate) file_module_membership: &'a FxHashMap<InternedPath, InternedPath>,
    pub(crate) module_root_public_exports:
        &'a FxHashMap<InternedPath, FxHashSet<PublicExportEntry>>,
}

/// Enforces module-private boundaries for cross-module-root dependencies.
///
/// WHAT: after a dependency resolves to a concrete source file, if the consumer and target are in
/// different module roots, the symbol must be exported by the target module's public root.
/// WHY: ordinary source declarations are bindable inside one module, but cross-module dependencies
/// must use the target module's explicit public export surface.
pub(crate) fn check_module_boundary(
    input: ModuleBoundaryCheckInput<'_>,
) -> BoundaryCheckResult<()> {
    let consumer_root = input.file_module_membership.get(input.consumer_file);
    let target_root = input.file_module_membership.get(input.target_file);

    // Skip if either file has no module root membership (e.g., source-backed packages handled separately).
    let (Some(consumer_root), Some(target_root)) = (consumer_root, target_root) else {
        return Ok(());
    };

    // Same module root: no boundary.
    if consumer_root == target_root {
        return Ok(());
    }

    // Different module roots: direct-selection public dependencies should already have resolved through the
    // target public surface. Direct source-path resolution here is therefore a boundary violation.
    if input.module_root_public_exports.contains_key(target_root) {
        return Err(Box::new(diagnostics::cross_module_dependency_not_exported(
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

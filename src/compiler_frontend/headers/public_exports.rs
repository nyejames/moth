//! Public export and file-membership data for header imports.
//!
//! WHAT: derives source-backed package and module-root public export maps from parsed headers and strict
//! `export:` block imports.
//! WHY: import environment preparation needs a single header-owned view of which declarations are
//! exposed across module-root boundaries and which source files belong to each boundary.
//!
//! ## Export map construction
//!
//! Public exports come from two sources:
//! 1. Public authored headers in the module-root file's `export:` block.
//! 2. Public grouped-import records from that same strict `export:` block.
//!
//! Because public imports may re-export symbols from other module roots, construction is
//! two-pass:
//! - Pass 1 collects all public authored declarations for every root file.
//! - Pass 2 resolves public imports against the completed authored export maps.

use crate::compiler_frontend::builtins::casts::traits::is_core_cast_trait_name;
use crate::compiler_frontend::compiler_errors::{CompilerError, compiler_error_to_diagnostic};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, ImportPublicSurfaceType, InvalidReceiverDeclarationReason,
    ReservedNameOwner,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::import_environment::{
    ExternalPackageSymbolLookup, ExternalPackageSymbolResolutionInput, ImportTargetResolutionInput,
    ModuleBoundaryCheckInput, PublicExportLookupResult, PublicExportResolutionInput,
    PublicExportSurfaceType, ResolvedImportTarget, SourcePackageBoundaryCheckInput,
    check_module_boundary, check_source_package_boundary, resolve_external_package_symbol,
    resolve_import_target, resolve_public_export_boundary,
};
use crate::compiler_frontend::headers::module_symbols::{
    ModuleRootBoundary, ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::types::{Header, HeaderExportMode};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderImportSet;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

/// Boxed diagnostic result for public export and membership construction.
///
/// WHAT: keeps the public export build/pass family on one small error boundary.
/// WHY: public export construction carries structured diagnostics through many successful
///      build steps without inlining the large diagnostic value at every return.
type PublicExportDataResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// Intern one filesystem-derived public-surface path without losing path components.
///
/// Public-export construction sees several logical, canonical and module-root paths, but they all
/// share the same exact-identity contract and infrastructure diagnostic lane.
fn intern_public_surface_path(
    path: &Path,
    path_role: &str,
    string_table: &mut StringTable,
) -> PublicExportDataResult<InternedPath> {
    InternedPath::try_from_filesystem_path(path, string_table).map_err(
        |NonUtf8PathComponent { path }| {
            Box::new(compiler_error_to_diagnostic(&CompilerError::file_error(
                &path,
                format!(
                    "{path_role} {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                ),
                string_table,
            )))
        },
    )
}

/// Whether a header is a public authored module-root export.
///
/// WHAT: only declarations marked public by a strict module-root file `export:` block become
///       public export entries. The declaration-kind gate is the shared
///       [`HeaderKind::is_authored_public_export_declaration`] owner so the header, AST and
///       semantic-origin public-export predicates cannot drift; this predicate keeps the
///       stage-local file-role and export-mode policy (export-capable roots plus explicit
///       `Public` mode).
fn is_authored_public_export(header: &Header) -> bool {
    header.file_role.is_export_capable()
        && header.export_mode == HeaderExportMode::Public
        && header.kind.is_authored_public_export_declaration()
}

/// Build public export maps and file package/module membership from parsed headers and the path
/// resolver.
pub(super) fn build_public_exports(
    module_symbols: &mut ModuleSymbols,
    headers: &[Header],
    resolver: &ProjectPathResolver,
    external_package_registry: &ExternalPackageRegistry,
    source_provider_imports: &SourceProviderImportSet<'_>,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    // Pass 1: collect public authored declarations for all root files.
    build_source_package_public_exports(module_symbols, headers, resolver, string_table)?;
    build_module_root_public_exports_pass1(module_symbols, headers, resolver, string_table)?;

    // Membership does not depend on import resolution.
    build_source_package_membership(module_symbols, resolver, string_table)?;
    build_module_root_membership(module_symbols, resolver, string_table)?;

    // Pass 2: resolve strict `export:` imports against the completed authored export maps.
    build_source_package_public_imports(
        module_symbols,
        resolver,
        external_package_registry,
        source_provider_imports,
        string_table,
    )?;
    build_module_root_public_imports(
        module_symbols,
        resolver,
        external_package_registry,
        source_provider_imports,
        string_table,
    )?;

    Ok(())
}

// --------------------------
//  Source-backed package public exports
// --------------------------

fn build_source_package_public_exports(
    module_symbols: &mut ModuleSymbols,
    headers: &[Header],
    resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    for (prefix, root_file) in resolver.source_package_public_surface_files() {
        let root_file_logical = resolver
            .logical_path_for_canonical_file(root_file, string_table)
            .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
        let root_file_interned = intern_public_surface_path(
            &root_file_logical,
            "Source package root file logical path",
            string_table,
        )?;

        let mut collector = PublicExportCollector::default();

        module_symbols
            .file_package_membership
            .insert(root_file_interned.clone(), prefix.clone());
        module_symbols
            .source_package_root_files
            .insert(prefix.clone(), root_file_interned.clone());

        for header in headers {
            if header.source_file != root_file_interned {
                continue;
            }

            if !is_authored_public_export(header) {
                continue;
            }

            if let Some(export_name) = header.tokens.src_path.name() {
                reject_source_receiver_method_export(
                    module_symbols,
                    &header.tokens.src_path,
                    header.name_location.clone(),
                )?;
                collector.insert(
                    export_name,
                    PublicExportTarget::Source(header.tokens.src_path.clone()),
                    header.name_location.clone(),
                    string_table,
                )?;
            }
        }

        module_symbols
            .source_package_public_exports
            .insert(prefix.clone(), collector.exports);
    }

    Ok(())
}

fn build_source_package_public_imports(
    module_symbols: &mut ModuleSymbols,
    resolver: &ProjectPathResolver,
    external_package_registry: &ExternalPackageRegistry,
    source_provider_imports: &SourceProviderImportSet<'_>,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    for (prefix, root_file) in resolver.source_package_public_surface_files() {
        let root_file_logical = resolver
            .logical_path_for_canonical_file(root_file, string_table)
            .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
        let root_file_interned = intern_public_surface_path(
            &root_file_logical,
            "Source package root file logical path",
            string_table,
        )?;

        let current_exports = module_symbols
            .source_package_public_exports
            .get(prefix)
            .cloned()
            .unwrap_or_default();
        let mut collector = PublicExportCollector::from_existing(&current_exports);

        if let Some(imports) = module_symbols
            .file_imports_by_source
            .get(&root_file_interned)
        {
            for import in imports {
                if import.export_mode != HeaderExportMode::Public {
                    continue;
                }

                let export_name = public_export_name(import)?;
                let target = resolve_public_export_import_or_provider(
                    module_symbols,
                    import,
                    &root_file_interned,
                    external_package_registry,
                    source_provider_imports,
                    string_table,
                )?;

                reject_public_export_target_if_source_receiver_method(
                    module_symbols,
                    &target,
                    import.location.clone(),
                )?;
                collector.insert(export_name, target, import.location.clone(), string_table)?;
            }
        }

        module_symbols
            .source_package_public_exports
            .insert(prefix.clone(), collector.exports);
    }

    Ok(())
}

// --------------------------
//  Module-root public exports
// --------------------------

fn build_module_root_public_exports_pass1(
    module_symbols: &mut ModuleSymbols,
    headers: &[Header],
    resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    let mut module_root_boundaries =
        build_module_root_boundaries(module_symbols, resolver, string_table)?;
    module_root_boundaries.sort_by_key(|boundary| std::cmp::Reverse(boundary.import_prefix.len()));
    module_symbols.module_root_boundaries = module_root_boundaries;

    for header in headers {
        let Some(canonical_path) = &header.tokens.canonical_os_path else {
            continue;
        };
        let Some(module_root) = resolver.module_root_for_file(canonical_path) else {
            continue;
        };

        let module_root_interned =
            intern_public_surface_path(&module_root, "Module root path", string_table)?;
        let logical = header.source_file.clone();
        let canonical = header.canonical_source_file(string_table);

        module_symbols
            .file_module_membership
            .insert(logical, module_root_interned.clone());
        module_symbols
            .file_module_membership
            .insert(canonical, module_root_interned.clone());

        if resolver
            .module_root_file_for_directory(&module_root)
            .is_some_and(|root_file| canonical_path.as_path() == root_file.as_path())
            && is_authored_public_export(header)
            && let Some(export_name) = header.tokens.src_path.name()
        {
            reject_source_receiver_method_export(
                module_symbols,
                &header.tokens.src_path,
                header.name_location.clone(),
            )?;
            let exports = module_symbols
                .module_root_public_exports
                .entry(module_root_interned)
                .or_default();
            exports.insert(PublicExportEntry {
                export_name,
                target: PublicExportTarget::Source(header.tokens.src_path.clone()),
            });
        }
    }

    Ok(())
}

fn build_module_root_public_imports(
    module_symbols: &mut ModuleSymbols,
    resolver: &ProjectPathResolver,
    external_package_registry: &ExternalPackageRegistry,
    source_provider_imports: &SourceProviderImportSet<'_>,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    let root_sources: Vec<_> = module_symbols
        .file_imports_by_source
        .keys()
        .filter(|source_file| {
            module_symbols
                .file_roles_by_source
                .get(*source_file)
                .is_some_and(|role| role.is_export_capable())
        })
        .cloned()
        .collect();

    for root_source in root_sources {
        let Some(canonical_export_path) =
            module_symbols.canonical_os_path_by_source.get(&root_source)
        else {
            continue;
        };
        let Some(module_root) = resolver.module_root_for_file(canonical_export_path) else {
            continue;
        };
        let Some(module_root_path) = resolver.module_root_file_for_directory(&module_root) else {
            continue;
        };

        if module_root_path != *canonical_export_path {
            continue;
        }

        let module_root_interned =
            intern_public_surface_path(&module_root, "Module root path", string_table)?;

        let current_exports = module_symbols
            .module_root_public_exports
            .get(&module_root_interned)
            .cloned()
            .unwrap_or_default();
        let mut collector = PublicExportCollector::from_existing(&current_exports);
        let imports = module_symbols
            .file_imports_by_source
            .get(&root_source)
            .cloned()
            .unwrap_or_default();

        for import in imports {
            if import.export_mode != HeaderExportMode::Public {
                continue;
            }

            let export_name = public_export_name(&import)?;
            let target = resolve_public_export_import_or_provider(
                module_symbols,
                &import,
                &root_source,
                external_package_registry,
                source_provider_imports,
                string_table,
            )?;

            reject_public_export_target_if_source_receiver_method(
                module_symbols,
                &target,
                import.location.clone(),
            )?;
            collector.insert(export_name, target, import.location.clone(), string_table)?;
        }

        module_symbols
            .module_root_public_exports
            .insert(module_root_interned.clone(), collector.exports);
    }

    Ok(())
}

fn resolve_public_export_import_or_provider(
    module_symbols: &ModuleSymbols,
    import: &crate::compiler_frontend::headers::types::FileImport,
    exporting_source: &InternedPath,
    external_package_registry: &ExternalPackageRegistry,
    source_provider_imports: &SourceProviderImportSet<'_>,
    string_table: &mut StringTable,
) -> PublicExportDataResult<PublicExportTarget> {
    if source_provider_imports
        .resolve(exporting_source, import, string_table)
        .is_some()
    {
        return Ok(PublicExportTarget::Source(import.provider.path.clone()));
    }

    resolve_public_export_import(
        module_symbols,
        import,
        exporting_source,
        external_package_registry,
        string_table,
    )
}

fn reject_public_export_target_if_source_receiver_method(
    module_symbols: &ModuleSymbols,
    target: &PublicExportTarget,
    location: SourceLocation,
) -> PublicExportDataResult<()> {
    let PublicExportTarget::Source(method_path) = target else {
        return Ok(());
    };

    reject_source_receiver_method_export(module_symbols, method_path, location)
}

fn reject_source_receiver_method_export(
    module_symbols: &ModuleSymbols,
    method_path: &InternedPath,
    location: SourceLocation,
) -> PublicExportDataResult<()> {
    if module_symbols.receiver_method_paths.contains(method_path) {
        return Err(Box::new(CompilerDiagnostic::invalid_receiver_declaration(
            InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
            location,
        )));
    }

    Ok(())
}

// --------------------------
//  Public import resolution
// --------------------------

/// Derive the public export name for a root-file import.
///
/// WHAT: alias wins; otherwise use the imported symbol name.
fn public_export_name(
    import: &crate::compiler_frontend::headers::parse_file_headers::FileImport,
) -> PublicExportDataResult<StringId> {
    match import.alias {
        Some(alias) => Ok(alias),
        None => match import.provider.path.name() {
            Some(name) => Ok(name),
            None => Err(Box::new(CompilerDiagnostic::missing_import_target(
                import.provider.path.clone(),
                import.location.clone(),
            ))),
        },
    }
}

/// Resolve a public import to its concrete export target.
///
/// WHAT: tries external package resolution, then public-boundary resolution, then direct source
///       resolution.
/// WHY: public imports in a root file re-export the resolved symbol through the module API.
fn resolve_public_export_import(
    module_symbols: &ModuleSymbols,
    import: &crate::compiler_frontend::headers::parse_file_headers::FileImport,
    root_file: &InternedPath,
    external_package_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> PublicExportDataResult<PublicExportTarget> {
    // 1. Try external package resolution first.
    match resolve_external_package_symbol(ExternalPackageSymbolResolutionInput {
        import_path: &import.provider.path,
        external_package_registry,
        string_table,
    }) {
        ExternalPackageSymbolLookup::Found { symbol_id } => {
            return Ok(PublicExportTarget::External(symbol_id));
        }
        ExternalPackageSymbolLookup::PackageFoundSymbolMissing {
            package_path,
            symbol_name,
        } => {
            return Err(Box::new(CompilerDiagnostic::missing_package_symbol(
                symbol_name,
                package_path,
                import.location.clone(),
            )));
        }
        ExternalPackageSymbolLookup::NoMatch => {}
    }

    // 2. Try public export boundary resolution.
    let public_boundary_input = PublicExportResolutionInput {
        importer_file: root_file,
        header_path: &import.provider.path,
        source_package_public_exports: &module_symbols.source_package_public_exports,
        file_package_membership: &module_symbols.file_package_membership,
        module_root_public_exports: &module_symbols.module_root_public_exports,
        file_module_membership: &module_symbols.file_module_membership,
        module_root_boundaries: &module_symbols.module_root_boundaries,
        string_table,
    };

    if let Some(public_boundary_result) = resolve_public_export_boundary(&public_boundary_input) {
        match public_boundary_result {
            PublicExportLookupResult::ExportedSource { path, .. } => {
                return Ok(PublicExportTarget::Source(path));
            }
            PublicExportLookupResult::ExportedExternal { symbol_id } => {
                return Ok(PublicExportTarget::External(symbol_id));
            }
            PublicExportLookupResult::NotExported {
                public_surface_name,
                public_surface_type,
            } => {
                // The entry-root public surface has no public path prefix. While building that root's
                // own public imports, root-relative same-module re-exports must still be allowed
                // to fall through to direct source resolution. Normal importers keep receiving
                // `NotExported` from `prepare_import_environment`.
                if matches!(public_surface_type, PublicExportSurfaceType::ModuleRoot)
                    && public_surface_name.is_empty()
                {
                    // Fall through to direct source resolution.
                } else {
                    // The target public surface exists but does not export this symbol.
                    // Preserve the same diagnostic that a normal importer would see.
                    let public_surface_name_id = string_table.intern(&public_surface_name);
                    let diagnostic_public_surface_type = match public_surface_type {
                        PublicExportSurfaceType::SourcePackage => {
                            ImportPublicSurfaceType::SourcePackage
                        }
                        PublicExportSurfaceType::ModuleRoot => ImportPublicSurfaceType::ModuleRoot,
                    };
                    return Err(Box::new(
                        CompilerDiagnostic::not_exported_by_public_surface(
                            import.provider.path.clone(),
                            public_surface_name_id,
                            diagnostic_public_surface_type,
                            import.location.clone(),
                        ),
                    ));
                }
            }
            PublicExportLookupResult::NotAPublicExportBoundary => {
                // Fall through to direct source resolution.
            }
        }
    }

    // 3. Direct source resolution.
    let target = resolve_import_target(ImportTargetResolutionInput {
        import_path: &import.provider.path,
        location: &import.location,
        module_file_paths: &module_symbols.module_file_paths,
        importable_symbol_paths: &module_symbols.importable_source_symbol_paths,
        external_package_registry,
        string_table,
    })?;

    match target {
        ResolvedImportTarget::Source { symbol_path, .. } => {
            if let Some(target_file) = module_symbols
                .canonical_source_by_symbol_path
                .get(&symbol_path)
            {
                check_source_package_boundary(SourcePackageBoundaryCheckInput {
                    importer_file: root_file,
                    target_file,
                    requested_path: &import.provider.path,
                    location: import.location.clone(),
                    file_package_membership: &module_symbols.file_package_membership,
                    source_package_root_files: &module_symbols.source_package_root_files,
                    string_table,
                })?;
                check_module_boundary(ModuleBoundaryCheckInput {
                    importer_file: root_file,
                    target_file,
                    symbol_path: &symbol_path,
                    location: import.location.clone(),
                    file_module_membership: &module_symbols.file_module_membership,
                    module_root_public_exports: &module_symbols.module_root_public_exports,
                })?;
            }

            Ok(PublicExportTarget::Source(symbol_path))
        }
        ResolvedImportTarget::External { symbol_id } => Ok(PublicExportTarget::External(symbol_id)),
    }
}

// --------------------------
//  Public export collection helper
// --------------------------

/// Accumulates public export entries for one root file and detects duplicate public names.
#[derive(Default)]
struct PublicExportCollector {
    exports: FxHashSet<PublicExportEntry>,
    seen_names: FxHashMap<StringId, SourceLocation>,
}

impl PublicExportCollector {
    fn from_existing(exports: &FxHashSet<PublicExportEntry>) -> Self {
        let mut seen_names = FxHashMap::default();
        for entry in exports {
            seen_names.insert(entry.export_name, SourceLocation::default());
        }
        Self {
            exports: exports.clone(),
            seen_names,
        }
    }

    fn insert(
        &mut self,
        export_name: StringId,
        target: PublicExportTarget,
        location: SourceLocation,
        string_table: &StringTable,
    ) -> PublicExportDataResult<()> {
        let export_name_text = string_table.resolve(export_name);
        if is_core_cast_trait_name(export_name_text) {
            return Err(Box::new(CompilerDiagnostic::reserved_name_collision(
                export_name,
                ReservedNameOwner::CoreTrait,
                location,
            )));
        }

        if self.seen_names.contains_key(&export_name) {
            return Err(Box::new(CompilerDiagnostic::duplicate_public_export(
                export_name,
                location,
            )));
        }
        self.seen_names.insert(export_name, location);
        self.exports.insert(PublicExportEntry {
            export_name,
            target,
        });
        Ok(())
    }
}

// --------------------------
//  Membership helpers
// --------------------------

fn build_source_package_membership(
    module_symbols: &mut ModuleSymbols,
    resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    for (source_file, canonical_path) in module_symbols.canonical_os_path_by_source.clone() {
        let Some((membership_prefix, _)) = resolver.source_package_for_file(&canonical_path) else {
            continue;
        };

        let canonical_source =
            intern_public_surface_path(&canonical_path, "Canonical source path", string_table)?;
        module_symbols
            .file_package_membership
            .insert(source_file.clone(), membership_prefix.to_owned());
        module_symbols
            .file_package_membership
            .insert(canonical_source, membership_prefix.to_owned());
    }

    Ok(())
}

fn build_module_root_membership(
    module_symbols: &mut ModuleSymbols,
    resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    for (source_file, canonical_path) in module_symbols.canonical_os_path_by_source.clone() {
        let Some(module_root) = resolver.module_root_for_file(&canonical_path) else {
            continue;
        };

        let module_root_interned =
            intern_public_surface_path(&module_root, "Module root path", string_table)?;
        let canonical_source =
            intern_public_surface_path(&canonical_path, "Canonical source path", string_table)?;

        module_symbols
            .file_module_membership
            .insert(source_file, module_root_interned.clone());
        module_symbols
            .file_module_membership
            .insert(canonical_source, module_root_interned);
    }

    Ok(())
}

fn build_module_root_boundaries(
    module_symbols: &mut ModuleSymbols,
    resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> PublicExportDataResult<Vec<ModuleRootBoundary>> {
    let mut module_root_boundaries = Vec::new();

    for module_root in resolver.module_roots() {
        let root_interned =
            intern_public_surface_path(module_root, "Module root path", string_table)?;

        let Some(root_file) = resolver.module_root_file_for_directory(module_root) else {
            continue;
        };
        module_symbols
            .module_root_public_exports
            .entry(root_interned.clone())
            .or_default();
        let root_file = resolver
            .logical_path_for_canonical_file(&root_file, string_table)
            .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
        let root_file =
            intern_public_surface_path(&root_file, "Module root file logical path", string_table)?;

        if let Ok(relative) = module_root.strip_prefix(resolver.entry_root()) {
            let prefix_interned = intern_public_surface_path(
                relative,
                "Module root relative prefix path",
                string_table,
            )?;
            module_root_boundaries.push(ModuleRootBoundary {
                import_prefix: prefix_interned,
                module_root: root_interned,
                root_file,
            });
        }
    }

    Ok(module_root_boundaries)
}

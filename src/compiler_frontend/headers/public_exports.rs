//! Public export and file-membership data for header dependencies.
//!
//! WHAT: derives source-backed package and module-root public export maps from parsed headers and strict
//! `export:` block dependencies.
//! WHY: header binding environment preparation needs a single header-owned view of which declarations are
//! exposed across module-root boundaries and which source files belong to each boundary.
//!
//! ## Export map construction
//!
//! Public exports come from two sources:
//! 1. Public authored headers in the module-root file's `export:` block.
//! 2. Public direct-selection dependency records from that same strict `export:` block.
//!
//! Because public dependencies may re-export symbols from other module roots, construction is
//! two-pass:
//! - Pass 1 collects all public authored declarations for every root file.
//! - Pass 2 resolves public dependencies against the completed authored export maps.

use crate::compiler_frontend::builtins::casts::traits::is_core_cast_trait_name;
use crate::compiler_frontend::compiler_errors::{CompilerError, compiler_error_to_diagnostic};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, ImportPublicSurfaceType, InvalidDependencyClauseReason,
    InvalidReceiverDeclarationReason, ReservedNameOwner,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::{
    DependencyTargetResolutionInput, ExternalPackageSymbolLookup,
    ExternalPackageSymbolResolutionInput, ModuleBoundaryCheckInput, PublicExportLookupResult,
    PublicExportResolutionInput, PublicExportSurfaceType, ResolvedDependencyTarget,
    SourcePackageBoundaryCheckInput, check_module_boundary, check_source_package_boundary,
    provider_public_surface_diagnostic, resolve_dependency_target, resolve_external_package_symbol,
    resolve_public_export_boundary,
};
use crate::compiler_frontend::headers::module_symbols::{
    ModuleRootBoundary, ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::types::{DependencySelection, Header, HeaderExportMode};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::project_globals::is_project_globals_dependency;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
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

/// Context for resolving one public dependency selection.
///
/// WHAT: carries the completed module facts and selected-name identity through provider-backed or
///       ordinary public-export resolution.
/// WHY: public-export construction has one semantic owner for this resolution boundary; keeping
///      its inputs named prevents the pass from growing a positional parameter list.
struct PublicExportDependencyResolutionInput<'a, 'provider> {
    module_symbols: &'a ModuleSymbols,
    dependency: &'a crate::compiler_frontend::headers::types::RetainedDependencyClause,
    selection: &'a DependencySelection,
    selection_index: usize,
    exporting_source: &'a InternedPath,
    external_package_registry: &'a ExternalPackageRegistry,
    source_provider_dependencies: &'a SourceProviderDependencySet<'provider>,
    string_table: &'a mut StringTable,
}

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
///       public export entries. The shared declaration gate also excludes source `#Config`
///       contract shells, which are resolved before ordinary declarations exist.
/// WHY: the stage-local file-role and export-mode policy remains here while declaration-kind and
///       contract-shell eligibility stay centralized on `HeaderKind`.
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
    source_provider_dependencies: &SourceProviderDependencySet<'_>,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    // Pass 1: collect public authored declarations for all root files.
    let source_package_locations =
        build_source_package_public_exports(module_symbols, headers, resolver, string_table)?;
    let module_root_locations =
        build_module_root_public_exports_pass1(module_symbols, headers, resolver, string_table)?;

    // Membership does not depend on dependency resolution.
    build_source_package_membership(module_symbols, resolver, string_table)?;
    build_module_root_membership(module_symbols, resolver, string_table)?;

    // Pass 2: resolve strict `export:` dependencies against the completed authored export maps.
    build_source_package_public_dependencies(
        module_symbols,
        &source_package_locations,
        resolver,
        external_package_registry,
        source_provider_dependencies,
        string_table,
    )?;
    build_module_root_public_dependencies(
        module_symbols,
        &module_root_locations,
        resolver,
        external_package_registry,
        source_provider_dependencies,
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
) -> PublicExportDataResult<FxHashMap<String, FxHashMap<StringId, SourceLocation>>> {
    let mut export_locations = FxHashMap::default();
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
        let mut root_locations = FxHashMap::default();

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
                    PublicExportTarget::SourceDeclaration {
                        path: header.tokens.src_path.clone(),
                    },
                    header.name_location.clone(),
                    string_table,
                )?;
                root_locations.insert(export_name, header.name_location.clone());
            }
        }

        export_locations.insert(prefix.clone(), root_locations);
        module_symbols
            .source_package_public_exports
            .insert(prefix.clone(), collector.exports);
    }

    Ok(export_locations)
}

fn build_source_package_public_dependencies(
    module_symbols: &mut ModuleSymbols,
    export_locations: &FxHashMap<String, FxHashMap<StringId, SourceLocation>>,
    resolver: &ProjectPathResolver,
    external_package_registry: &ExternalPackageRegistry,
    source_provider_dependencies: &SourceProviderDependencySet<'_>,
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
        let mut collector =
            PublicExportCollector::from_existing(&current_exports, export_locations.get(prefix));

        if let Some(dependencies) = module_symbols
            .file_dependency_clauses_by_source
            .get(&root_file_interned)
        {
            for dependency in dependencies {
                if dependency.export_mode != HeaderExportMode::Public {
                    continue;
                }

                let selections = module_symbols
                    .selections_for_clause(&root_file_interned, dependency)
                    .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
                for (selection_index, selection) in selections.iter().enumerate() {
                    let export_name = public_export_name(selection);
                    let target = resolve_public_export_dependency_or_provider(
                        PublicExportDependencyResolutionInput {
                            module_symbols,
                            dependency,
                            selection,
                            selection_index,
                            exporting_source: &root_file_interned,
                            external_package_registry,
                            source_provider_dependencies,
                            string_table,
                        },
                    )?;

                    reject_public_export_target_if_source_receiver_method(
                        module_symbols,
                        &target,
                        selection.source_location.clone(),
                    )?;
                    collector.insert(
                        export_name,
                        target,
                        selection
                            .local_alias()
                            .map_or(&selection.source_location, |alias| &alias.location)
                            .clone(),
                        string_table,
                    )?;
                }
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
) -> PublicExportDataResult<FxHashMap<InternedPath, FxHashMap<StringId, SourceLocation>>> {
    let mut export_locations = FxHashMap::default();
    let mut module_root_boundaries =
        build_module_root_boundaries(module_symbols, resolver, string_table)?;
    module_root_boundaries
        .sort_by_key(|boundary| std::cmp::Reverse(boundary.dependency_prefix.len()));
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
                .entry(module_root_interned.clone())
                .or_default();
            exports.insert(PublicExportEntry {
                export_name,
                target: PublicExportTarget::SourceDeclaration {
                    path: header.tokens.src_path.clone(),
                },
            });
            export_locations
                .entry(module_root_interned)
                .or_insert_with(FxHashMap::default)
                .insert(export_name, header.name_location.clone());
        }
    }

    Ok(export_locations)
}

fn build_module_root_public_dependencies(
    module_symbols: &mut ModuleSymbols,
    export_locations: &FxHashMap<InternedPath, FxHashMap<StringId, SourceLocation>>,
    resolver: &ProjectPathResolver,
    external_package_registry: &ExternalPackageRegistry,
    source_provider_dependencies: &SourceProviderDependencySet<'_>,
    string_table: &mut StringTable,
) -> PublicExportDataResult<()> {
    let root_sources: Vec<_> = module_symbols
        .file_dependency_clauses_by_source
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
        let dependencies = module_symbols
            .file_dependency_clauses_by_source
            .get(&root_source)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for dependency in dependencies {
            if dependency.export_mode != HeaderExportMode::Public
                || !is_project_globals_dependency(&dependency.dependency.path, string_table)
            {
                continue;
            }
            let clause_kind = dependency.binding.clause_kind();
            return Err(Box::new(CompilerDiagnostic::invalid_dependency_clause(
                clause_kind,
                InvalidDependencyClauseReason::ProjectGlobalsReexportNotAllowed,
                dependency.location.clone(),
            )));
        }

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
        let mut collector = PublicExportCollector::from_existing(
            &current_exports,
            export_locations.get(&module_root_interned),
        );

        for dependency in dependencies {
            if dependency.export_mode != HeaderExportMode::Public {
                continue;
            }

            let selections = module_symbols
                .selections_for_clause(&root_source, dependency)
                .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
            for (selection_index, selection) in selections.iter().enumerate() {
                let export_name = public_export_name(selection);
                let target = resolve_public_export_dependency_or_provider(
                    PublicExportDependencyResolutionInput {
                        module_symbols,
                        dependency,
                        selection,
                        selection_index,
                        exporting_source: &root_source,
                        external_package_registry,
                        source_provider_dependencies,
                        string_table,
                    },
                )?;

                reject_public_export_target_if_source_receiver_method(
                    module_symbols,
                    &target,
                    selection.source_location.clone(),
                )?;
                collector.insert(
                    export_name,
                    target,
                    selection
                        .local_alias()
                        .map_or(&selection.source_location, |alias| &alias.location)
                        .clone(),
                    string_table,
                )?;
            }
        }

        module_symbols
            .module_root_public_exports
            .insert(module_root_interned.clone(), collector.exports);
    }

    Ok(())
}

fn resolve_public_export_dependency_or_provider(
    input: PublicExportDependencyResolutionInput<'_, '_>,
) -> PublicExportDataResult<PublicExportTarget> {
    let PublicExportDependencyResolutionInput {
        module_symbols,
        dependency,
        selection,
        selection_index,
        exporting_source,
        external_package_registry,
        source_provider_dependencies,
        string_table,
    } = input;

    if let Some(resolved_clause) =
        source_provider_dependencies.resolve_clause(dependency.dependency.dependency_shell_id)
    {
        let provider_id = resolved_clause.provider;
        let provider_name = string_table.resolve(selection.source_name);
        let provider_view = source_provider_dependencies
            .binding_view(provider_id)
            .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
        let diagnostic_path = dependency.dependency.path.append(selection.source_name);

        if provider_view.exported_origin(provider_name).is_none()
            && provider_view.binding_export(provider_name).is_none()
        {
            let interface = source_provider_dependencies
                .interface(provider_id)
                .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;
            return Err(Box::new(provider_public_surface_diagnostic(
                &diagnostic_path,
                interface,
                selection.source_location.clone(),
                string_table,
            )));
        }

        return Ok(PublicExportTarget::ProviderSelection {
            selection: dependency
                .selection_id(
                    module_symbols
                        .dependency_selections_by_source
                        .get(exporting_source)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    selection_index,
                )
                .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?,
            source_name: selection.source_name,
            diagnostic_path,
        });
    }

    resolve_public_export_dependency(
        module_symbols,
        dependency,
        selection,
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
    match target {
        PublicExportTarget::SourceDeclaration { path } => {
            reject_source_receiver_method_export(module_symbols, path, location)
        }
        // Provider selections are validated before their tagged target is retained. The later
        // namespace consumer repeats the member check as an internal invariant at its binding
        // boundary, where malformed retained state must still fail closed.
        PublicExportTarget::ProviderSelection { .. } => Ok(()),
        PublicExportTarget::External(_) => Ok(()),
    }
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
//  Public dependency resolution
// --------------------------

/// Derive the public export name for a root-file dependency.
///
/// WHAT: a direct selection alias wins; otherwise the selected provider-surface name is exported.
fn public_export_name(selection: &DependencySelection) -> StringId {
    selection.local_name()
}

/// Resolve a public dependency to its concrete export target.
///
/// WHAT: tries external package resolution, then public-boundary resolution, then direct source
///       resolution.
/// WHY: public dependencies in a root file re-export the resolved symbol through the module API.
fn resolve_public_export_dependency(
    module_symbols: &ModuleSymbols,
    dependency: &crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause,
    selection: &DependencySelection,
    root_file: &InternedPath,
    external_package_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> PublicExportDataResult<PublicExportTarget> {
    let selected_path = dependency.dependency.path.append(selection.source_name);

    // 1. Try external package resolution first.
    match resolve_external_package_symbol(ExternalPackageSymbolResolutionInput {
        dependency_path: &selected_path,
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
                selection.source_location.clone(),
            )));
        }
        ExternalPackageSymbolLookup::NoMatch => {}
    }

    // 2. Try public export boundary resolution.
    let public_boundary_input = PublicExportResolutionInput {
        consumer_file: root_file,
        header_path: &selected_path,
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
                return Ok(PublicExportTarget::SourceDeclaration { path });
            }
            PublicExportLookupResult::ExportedProviderSelection {
                selection,
                source_name,
                diagnostic_path,
                ..
            } => {
                return Ok(PublicExportTarget::ProviderSelection {
                    selection,
                    source_name,
                    diagnostic_path,
                });
            }
            PublicExportLookupResult::ExportedExternal { symbol_id } => {
                return Ok(PublicExportTarget::External(symbol_id));
            }
            PublicExportLookupResult::NotExported {
                public_surface_name,
                public_surface_type,
            } => {
                // The entry-root public surface has no public path prefix. While building that root's
                // own public dependencies, root-relative same-module re-exports must still be allowed
                // to fall through to direct source resolution. Normal consumers keep receiving
                // `NotExported` from `prepare_binding_environment`.
                if matches!(public_surface_type, PublicExportSurfaceType::ModuleRoot)
                    && public_surface_name.is_empty()
                {
                    // Fall through to direct source resolution.
                } else {
                    // The target public surface exists but does not export this symbol.
                    // Preserve the same diagnostic that a normal declaring_source would see.
                    let public_surface_name_id = string_table.intern(&public_surface_name);
                    let diagnostic_public_surface_type = match public_surface_type {
                        PublicExportSurfaceType::SourcePackage => {
                            ImportPublicSurfaceType::SourcePackage
                        }
                        PublicExportSurfaceType::ModuleRoot => ImportPublicSurfaceType::ModuleRoot,
                    };
                    return Err(Box::new(
                        CompilerDiagnostic::not_exported_by_public_surface(
                            selected_path.clone(),
                            public_surface_name_id,
                            diagnostic_public_surface_type,
                            selection.source_location.clone(),
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
    let target = resolve_dependency_target(DependencyTargetResolutionInput {
        dependency_path: &selected_path,
        location: &selection.source_location,
        module_file_paths: &module_symbols.module_file_paths,
        dependency_bindable_symbol_paths: &module_symbols.dependency_bindable_source_symbol_paths,
        external_package_registry,
        string_table,
    })?;

    match target {
        ResolvedDependencyTarget::Source { symbol_path, .. } => {
            if let Some(target_file) = module_symbols
                .canonical_source_by_symbol_path
                .get(&symbol_path)
            {
                check_source_package_boundary(SourcePackageBoundaryCheckInput {
                    consumer_file: root_file,
                    target_file,
                    requested_path: &selected_path,
                    location: selection.source_location.clone(),
                    file_package_membership: &module_symbols.file_package_membership,
                    source_package_root_files: &module_symbols.source_package_root_files,
                    string_table,
                })?;
                check_module_boundary(ModuleBoundaryCheckInput {
                    consumer_file: root_file,
                    target_file,
                    symbol_path: &symbol_path,
                    location: selection.source_location.clone(),
                    file_module_membership: &module_symbols.file_module_membership,
                    module_root_public_exports: &module_symbols.module_root_public_exports,
                })?;
            }

            Ok(PublicExportTarget::SourceDeclaration { path: symbol_path })
        }
        ResolvedDependencyTarget::External { symbol_id } => {
            Ok(PublicExportTarget::External(symbol_id))
        }
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
    fn from_existing(
        exports: &FxHashSet<PublicExportEntry>,
        existing_locations: Option<&FxHashMap<StringId, SourceLocation>>,
    ) -> Self {
        let mut seen_names = FxHashMap::default();
        for entry in exports {
            seen_names.insert(
                entry.export_name,
                existing_locations
                    .and_then(|locations| locations.get(&entry.export_name))
                    .cloned()
                    .unwrap_or_default(),
            );
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

        if let Some(first_location) = self.seen_names.get(&export_name) {
            return Err(Box::new(CompilerDiagnostic::duplicate_public_export(
                export_name,
                first_location.clone(),
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
                dependency_prefix: prefix_interned,
                module_root: root_interned,
                root_file,
            });
        }
    }

    Ok(module_root_boundaries)
}

#[cfg(test)]
#[path = "tests/public_exports_tests.rs"]
mod tests;

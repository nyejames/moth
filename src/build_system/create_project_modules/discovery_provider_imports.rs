use super::*;
// -------------------------
//  Provider-backed import resolution
// -------------------------

pub(super) struct ProviderBackedImportRequest<'a> {
    pub(super) consumer_canonical_path: &'a Path,
    pub(super) import_path: &'a InternedPath,
    pub(super) source_span: Option<SourceSpan>,
    pub(super) prefix_path: &'a InternedPath,
    pub(super) raw_prefix: &'a str,
    pub(super) provider: &'a std::sync::Arc<dyn ExternalImportProvider>,
    pub(super) project_path_resolver: &'a ProjectPathResolver,
    pub(super) directory_dependency_resolution: Option<DirectoryDependencyResolution<'a>>,
}

/// Resolves a provider-backed import prefix to a canonical filesystem path, checks the build cache,
/// calls the provider if needed, and records the result in the resolution table and package registry.
pub(super) fn resolve_provider_backed_import(
    request: ProviderBackedImportRequest<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    // Directory projects resolve provider-owned targets through the same boundary-aware
    // namespace as compiler-semantic dependencies. Single-file synthetic compilation retains its
    // separate filesystem-backed resolver.
    let canonical_source_path = match request.directory_dependency_resolution {
        Some(resolution) => resolution
            .resolve_provider_target(
                request.prefix_path,
                request.consumer_canonical_path,
                request.source_span,
                string_table,
            )
            .map_err(SourceDiscoveryError::from)?,
        None => resolve_provider_target_via_filesystem(&request, string_table)?,
    };

    invoke_provider_and_record_resolution(
        canonical_source_path,
        &request,
        external_imports,
        string_table,
    )
}

/// Resolve a single-file provider import target through the filesystem.
///
/// WHAT: the single-file synthetic-module mode retains its filesystem-backed provider
/// resolution, canonicalizing the normalized candidate and checking the module boundary from
/// the resolver's root tables. This is deliberately separate from the directory index path.
fn resolve_provider_target_via_filesystem(
    request: &ProviderBackedImportRequest<'_>,
    string_table: &mut StringTable,
) -> Result<PathBuf, SourceDiscoveryError> {
    let canonical_source_path = resolve_provider_prefix_to_canonical_path(
        request.prefix_path,
        request.consumer_canonical_path,
        request.project_path_resolver,
        string_table,
    )?;

    // Enforce module/package boundaries for provider-backed imports.
    check_provider_dependency_module_boundary(
        request.consumer_canonical_path,
        &canonical_source_path,
        request.import_path,
        request.source_span,
        request.project_path_resolver,
    )?;

    Ok(canonical_source_path)
}

/// Check the build cache, call the provider when needed, and record the result in the
/// resolution table and package registry.
///
/// WHAT: shared between the directory index path and the single-file filesystem path. The
/// `canonical_source_path` is the resolved target's IO handle from either path.
fn invoke_provider_and_record_resolution(
    canonical_source_path: PathBuf,
    request: &ProviderBackedImportRequest<'_>,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    string_table: &mut StringTable,
) -> Result<(), SourceDiscoveryError> {
    let cache_key = ExternalImportCacheKey {
        canonical_source_path: canonical_source_path.clone(),
        provider_kind: request.provider.kind(),
    };

    // Use cached result when available.
    if let Some(cached) = external_imports.cache.get(&cache_key) {
        let source_file_logical = source_file_logical_path(
            request.consumer_canonical_path,
            request.project_path_resolver,
        )?;
        external_imports.resolution_table.insert(
            source_file_logical,
            request.raw_prefix,
            cached.clone(),
        );
        return Ok(());
    }

    // The provider request carries the portable logical spelling so stable package and asset
    // identity never keys on this machine's checkout path.
    let logical_source = request
        .project_path_resolver
        .logical_path_for_canonical_file(&canonical_source_path)
        .map_err(SourceDiscoveryError::from)?;
    let logical_source_path = PortableResourcePath::from_relative_logical_path(&logical_source)
        .map_err(SourceDiscoveryError::from)?;

    let provider_request = ExternalImportRequest {
        import_path: request.import_path.to_portable_string(string_table),
        logical_source_path,
        canonical_source_path: canonical_source_path.clone(),
        source_span: request.source_span,
    };

    let result = {
        let mut context = ExternalImportProviderContext {
            package_registry: external_imports.external_packages,
            cache: external_imports.cache,
            string_table,
        };

        request
            .provider
            .resolve_external_import(provider_request, &mut context)
            .map_err(SourceDiscoveryError::from)?
    };

    if let Some(resolved) = result {
        external_imports.cache.insert(cache_key, resolved.clone());

        let source_file_logical = source_file_logical_path(
            request.consumer_canonical_path,
            request.project_path_resolver,
        )?;
        external_imports
            .resolution_table
            .insert(source_file_logical, request.raw_prefix, resolved);
    }

    Ok(())
}

/// Resolves a provider import prefix to a canonical filesystem path without selecting a compiler
/// source extension candidate.
///
/// WHAT: reuses the normal base/boundary/case rules from `ProjectPathResolver` but skips the
/// extension candidate selection used by isolated compiler-source resolution.
fn resolve_provider_prefix_to_canonical_path(
    prefix_path: &InternedPath,
    declaring_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<PathBuf, SourceDiscoveryError> {
    let (base_kind, filesystem_base) = if let Some(package_root) =
        project_path_resolver.source_package_root_for_dependency(prefix_path, string_table)
    {
        (
            crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase::SourcePackageRoot,
            package_root,
        )
    } else {
        // Dependency paths are module-root-relative. Synthetic traversal has no indexed module
        // namespace, so it derives the same owning boundary from the resolver's retained module
        // roots and uses the entry root only for the default module.
        let module_root = project_path_resolver
            .module_root_for_file(declaring_file)
            .unwrap_or_else(|| project_path_resolver.entry_root().to_path_buf());
        (
            crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase::EntryRoot,
            module_root,
        )
    };

    let normalized = join_and_normalize_path(&filesystem_base, prefix_path, string_table);

    let canonical = fs::canonicalize(&normalized)
        .map_err(|error| {
            CompilerError::file_error(
                declaring_file,
                format!(
                    "Failed to canonicalize external import prefix '{}': {error}",
                    normalized.display()
                ),
            )
        })
        .map_err(SourceDiscoveryError::from)?;

    crate::compiler_frontend::paths::dependency_resolution::validate_dependency_boundary(
        &canonical,
        &base_kind,
        &filesystem_base,
        prefix_path,
    )
    .map_err(SourceDiscoveryError::from)?;

    Ok(canonical)
}

/// Derives the portable logical path for a canonical source file.
fn source_file_logical_path(
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
) -> Result<String, SourceDiscoveryError> {
    let logical = project_path_resolver
        .logical_path_for_canonical_file(canonical_file)
        .map_err(SourceDiscoveryError::from)?;
    let logical_text = logical.to_str().ok_or_else(|| {
        SourceDiscoveryError::from(CompilerError::file_error(
            &logical,
            format!(
                "Source file logical path {logical:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
            ),
        ))
    })?;
    Ok(logical_text.replace('\\', "/"))
}

// -------------------------
//  Provider import boundary check
// -------------------------

/// Enforce that a provider-backed dependency does not cross a module or source-backed package boundary.
///
/// WHAT: .js files are private implementation details of the module or package that owns them.
///       Cross-module or cross-package .js dependencies bypass the public surface and are rejected.
/// WHY: provider-backed dependencies must obey the same visibility boundaries as .moth source dependencies.
fn check_provider_dependency_module_boundary(
    declaring_file: &Path,
    target_file: &Path,
    dependency_path: &InternedPath,
    source_span: Option<SourceSpan>,
    project_path_resolver: &ProjectPathResolver,
) -> Result<(), SourceDiscoveryError> {
    let consumer_container = provider_dependency_container(project_path_resolver, declaring_file);
    let target_container = provider_dependency_container(project_path_resolver, target_file);

    if consumer_container != target_container {
        return Err(SourceDiscoveryError::from(
            CompilerDiagnostic::cross_module_import_not_exported(
                dependency_path.clone(),
                source_span,
            ),
        ));
    }

    Ok(())
}

/// Determine the boundary "container" of a file for provider import checks.
///
/// WHAT: returns the module root, source-backed package root, or entry root that contains the file.
/// WHY: two files in the same container may freely import each other's .js files.
fn provider_dependency_container(
    project_path_resolver: &ProjectPathResolver,
    file: &Path,
) -> Option<PathBuf> {
    // Module roots are the most specific boundaries.
    if let Some(root) = project_path_resolver.module_root_for_file(file) {
        return Some(root);
    }

    // Source-backed packages are the next boundary. Use the resolver's nearest-root policy so nested
    // packages do not inherit provider access from an outer registered root.
    if let Some((_, root)) = project_path_resolver.source_package_for_file(file) {
        return Some(root.to_path_buf());
    }

    // Everything under the entry root belongs to the default module.
    if file.starts_with(project_path_resolver.entry_root()) {
        return Some(project_path_resolver.entry_root().to_path_buf());
    }

    None
}

// -------------------------
//  Diagnostic Helpers
// -------------------------

pub(super) fn unsupported_builder_package_error(
    package_path: &str,
    source_span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let package_path_id = string_table.intern(package_path);
    CompilerDiagnostic::unsupported_builder_package(package_path_id, source_span)
}

pub(super) fn unsupported_external_extension_error(
    import_path: &InternedPath,
    extension: &str,
    source_span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let extension_id = string_table.intern(extension);
    CompilerDiagnostic::unsupported_external_extension(
        import_path.clone(),
        extension_id,
        source_span,
    )
}

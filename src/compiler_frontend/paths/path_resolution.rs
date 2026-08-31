//! Project-aware compile-time path and single-file dependency resolution.
//!
//! `ProjectPathResolver` keeps the public resolution surface for Stage 0, headers, AST folding,
//! and builder-facing path tracking. The data contracts, module-root scanning, and path
//! normalization helpers live in sibling modules so this file can focus on orchestration and
//! diagnostic boundaries.

use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCompileTimePathReason, InvalidImportPathReason,
};
use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePath, CompileTimePathBase, CompileTimePathResolutionError,
    validate_path_literal_target,
};
use crate::compiler_frontend::paths::dependency_resolution::{
    DependencyPathResolutionError, validate_dependency_boundary,
    validate_dependency_case_sensitivity,
};
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_normalization::{
    DependencyCandidate, DependencyCandidateSupport, build_public_path,
    candidate_dependency_files_for_source_kinds, canonicalize_best_effort,
    dependency_contains_dotdot, is_relative_dependency_path, join_and_normalize_path,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::{Path, PathBuf};

/// Concrete source-file dependency selected by path resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedDependencyFile {
    pub(crate) path: PathBuf,
    pub(crate) kind: SourceFileKind,
}

/// WHAT: resolves project-aware dependency paths using the configured entry root and source-backed packages.
/// WHY: Stage 0 discovery and later frontend dependency normalization must use identical path rules.
#[derive(Clone, Debug)]
pub(crate) struct ProjectPathResolver {
    project_root: PathBuf,
    entry_root: PathBuf,
    /// Canonical source-backed package roots and their prepared public surfaces from Stage 0.
    source_package_roots: PreparedSourcePackageRoots,
    /// Module roots prepared by Stage 0. Resolver construction never discovers them.
    module_roots: ModuleRootTable,
    /// Builder-supported source file kinds available for this project.
    source_file_kinds: SourceFileKindRegistry,
}

impl ProjectPathResolver {
    /// WHAT: creates a resolver from canonical project and entry roots.
    /// WHY: dependency normalization depends on a stable filesystem view of the project layout.
    pub(crate) fn new(
        project_root: PathBuf,
        entry_root: PathBuf,
        source_package_roots: PreparedSourcePackageRoots,
        source_file_kinds: &SourceFileKindRegistry,
    ) -> Result<Self, CompilerError> {
        Self::new_with_module_roots(
            project_root,
            entry_root,
            source_package_roots,
            source_file_kinds,
            ModuleRootTable::empty(),
        )
    }

    /// WHAT: creates a resolver from canonical roots and Stage 0 module-root data.
    /// WHY: path resolution may query prepared module boundaries, but it must not perform
    /// filesystem discovery during normal directory construction.
    pub(crate) fn new_with_module_roots(
        project_root: PathBuf,
        entry_root: PathBuf,
        source_package_roots: PreparedSourcePackageRoots,
        source_file_kinds: &SourceFileKindRegistry,
        module_roots: ModuleRootTable,
    ) -> Result<Self, CompilerError> {
        Ok(Self {
            project_root,
            entry_root,
            source_package_roots,
            module_roots,
            source_file_kinds: source_file_kinds.clone(),
        })
    }

    /// Derive a resolver scoped to one independently compiled source-package boundary.
    ///
    /// Source/package dependencies still use the shared indexed namespace and registered package
    /// surfaces. Compile-time paths, root membership and portable source paths use the package's
    /// own root so a Builder or dependency package never acquires the consuming project's path
    /// context.
    pub(crate) fn for_source_package_boundary(
        &self,
        package_root: PathBuf,
        module_roots: ModuleRootTable,
    ) -> Self {
        let mut resolver = self.clone();
        resolver.project_root = package_root.clone();
        resolver.entry_root = package_root;
        resolver.module_roots = module_roots;
        resolver
    }

    /// WHAT: exposes the canonical entry root for module discovery and diagnostics.
    /// WHY: callers need one canonical source of truth after config parsing.
    pub(crate) fn entry_root(&self) -> &Path {
        &self.entry_root
    }

    /// WHAT: returns the map of source-backed package roots.
    #[cfg(test)]
    pub(crate) fn source_package_roots(&self) -> &std::collections::BTreeMap<String, PathBuf> {
        self.source_package_roots.roots()
    }

    /// Return the most-specific source-backed package containing a canonical file path.
    ///
    /// WHAT: selects the deepest matching source-backed package root, then the smallest dependency prefix
    ///       when multiple prefixes name the same root.
    /// WHY: logical paths, header membership and provider boundaries must share one deterministic
    ///      owner when registered source-backed package roots overlap.
    pub(crate) fn source_package_for_file(&self, file: &Path) -> Option<(&str, &Path)> {
        let mut nearest_package: Option<(&str, &Path)> = None;

        for (prefix, root) in self.source_package_roots.roots() {
            if !file.starts_with(root) {
                continue;
            }

            let should_replace = match nearest_package {
                None => true,
                Some((nearest_prefix, nearest_root)) => {
                    let root_depth = root.components().count();
                    let nearest_depth = nearest_root.components().count();

                    root_depth > nearest_depth
                        || (root_depth == nearest_depth && prefix.as_str() < nearest_prefix)
                }
            };

            if should_replace {
                nearest_package = Some((prefix.as_str(), root.as_path()));
            }
        }

        nearest_package
    }

    /// Return the prepared source-package root selected by an authored dependency prefix.
    ///
    /// WHAT: exposes the same longest-prefix package classification used by ordinary dependency
    ///       resolution to Stage 0's synthetic module-root precedence check.
    /// WHY: registered package paths must retain package-root semantics and must not be mistaken
    ///      for bare module-root-relative source paths.
    pub(crate) fn source_package_root_for_dependency(
        &self,
        dependency_path: &InternedPath,
        string_table: &StringTable,
    ) -> Option<PathBuf> {
        self.matches_source_package_prefix(dependency_path, string_table)
    }

    /// Returns each source-backed package's unique normal-root file as its public surface.
    pub(crate) fn source_package_public_surface_files(
        &self,
    ) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.source_package_roots.root_files().iter()
    }

    pub(crate) fn module_root_file_for_directory(&self, directory: &Path) -> Option<PathBuf> {
        self.module_roots
            .root_file_for_directory(directory)
            .map(|path| path.to_path_buf())
    }

    pub(crate) fn is_module_root_file(&self, file: &Path) -> bool {
        self.module_roots.is_root_file(file)
    }

    pub(crate) fn module_roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.module_roots.root_directories()
    }

    /// WHAT: returns the builder-supported source file kinds for this project.
    /// WHY: Stage 0 and dependency resolution need to know which non-`.moth` extensions are valid.
    pub(crate) fn source_file_kinds(&self) -> &SourceFileKindRegistry {
        &self.source_file_kinds
    }

    /// WHAT: returns the module root that contains the given file.
    /// WHY: nearest-ancestor lookup determines which module a file belongs to.
    pub(crate) fn module_root_for_file(&self, file: &Path) -> Option<PathBuf> {
        self.module_roots.module_root_for_file(file)
    }

    /// WHAT: derive a portable logical source path from a canonical filesystem file path.
    /// WHY: frontend identity should preserve dependency semantics without leaking machine-local paths.
    ///
    /// NOTE: `string_table` is only used on error paths to intern diagnostic file paths.
    pub(crate) fn logical_path_for_canonical_file(
        &self,
        canonical_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<PathBuf, CompilerError> {
        if let Ok(relative_to_entry_root) = canonical_file.strip_prefix(&self.entry_root) {
            return Ok(relative_to_entry_root.to_path_buf());
        }

        if let Ok(relative_to_project_root) = canonical_file.strip_prefix(&self.project_root) {
            return Ok(relative_to_project_root.to_path_buf());
        }

        // Source-backed package files may live outside the project root (builder-provided).
        // Derive a logical path from the same nearest-root policy used by membership checks.
        if let Some((prefix, root)) = self.source_package_for_file(canonical_file)
            && let Ok(relative_to_package_root) = canonical_file.strip_prefix(root)
        {
            let mut logical = PathBuf::from(prefix);
            logical.push(relative_to_package_root);
            return Ok(logical);
        }

        Err(CompilerError::file_error(
            canonical_file,
            format!(
                "Source file '{}' is outside both entry root '{}' and project root '{}'",
                canonical_file.display(),
                self.entry_root.display(),
                self.project_root.display()
            ),
            string_table,
        ))
    }

    /// WHAT: resolves a dependency path to a concrete source file and its source kind.
    /// WHY: Stage 0 must preserve the source kind so `.mtf` files can be discovered without being
    ///      scanned or prepared as normal Moth source.
    pub(crate) fn resolve_dependency_to_source_file(
        &self,
        dependency_path: &InternedPath,
        declaring_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<ResolvedDependencyFile, DependencyPathResolutionError> {
        let (_, canonical) = self.resolve_dependency_as_compile_time_path(
            dependency_path,
            declaring_file,
            string_table,
        )?;
        let source_kind = self
            .source_kind_for_canonical_path(&canonical)
            .ok_or_else(|| {
                CompilerError::file_error(
                    declaring_file,
                    format!(
                        "Resolved dependency '{}' to '{}' but could not determine its source kind.",
                        dependency_path.to_portable_string(string_table),
                        canonical.display()
                    ),
                    string_table,
                )
            })?;

        Ok(ResolvedDependencyFile {
            path: canonical,
            kind: source_kind,
        })
    }

    /// WHAT: resolves one dependency path to both a typed compile-time path and a canonical file path.
    /// WHY: dependencies use the same resolution model as general path literals, but additionally
    ///      apply `.moth` extension fallback logic. Returns both representations so callers
    ///      can choose what they need.
    ///
    /// NOTE: `string_table` is used for diagnostic path interning and case-mismatch strings.
    pub(crate) fn resolve_dependency_as_compile_time_path(
        &self,
        dependency_path: &InternedPath,
        declaring_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<(CompileTimePath, PathBuf), DependencyPathResolutionError> {
        if let Some(extension) = explicit_source_extension(dependency_path, string_table) {
            let location = SourceLocation::from_path(declaring_file, string_table);
            let diagnostic = if extension == SourceFileKind::Moth.extension() {
                CompilerDiagnostic::explicit_moth_extension(dependency_path.to_owned(), location)
            } else {
                let extension_id = string_table.intern(&extension);
                CompilerDiagnostic::explicit_source_extension(
                    dependency_path.to_owned(),
                    extension_id,
                    location,
                )
            };
            return Err(DependencyPathResolutionError::Diagnostic(Box::new(
                diagnostic,
            )));
        }

        if dependency_contains_dotdot(dependency_path, string_table) {
            let location = SourceLocation::from_path(declaring_file, string_table);
            let diagnostic = CompilerDiagnostic::invalid_import_path(
                dependency_path.to_owned(),
                InvalidImportPathReason::ParentDirectorySegment,
                location,
            );
            return Err(DependencyPathResolutionError::Diagnostic(Box::new(
                diagnostic,
            )));
        }

        let (base_kind, filesystem_base) =
            self.resolve_path_base(dependency_path, declaring_file, string_table)?;

        // Source-backed package roots already include the prefix directory, so skip the first
        // component when joining to avoid double-prefixing (e.g. `lib/helper/helper/...`).
        let normalized = if matches!(base_kind, CompileTimePathBase::SourcePackageRoot) {
            let components = dependency_path.as_components();
            let suffix = if components.len() <= 1 {
                InternedPath::new()
            } else {
                InternedPath::from_components(components[1..].to_vec())
            };
            join_and_normalize_path(&filesystem_base, &suffix, string_table)
        } else {
            join_and_normalize_path(&filesystem_base, dependency_path, string_table)
        };

        let candidates = candidate_dependency_files_for_source_kinds(
            &normalized,
            dependency_path.len(),
            self.source_file_kinds(),
        );
        let existing_candidates = existing_dependency_candidates(&candidates);
        let folder_exists = normalized.is_dir();

        if existing_candidates.len() + usize::from(folder_exists) > 1 {
            let location = SourceLocation::from_path(declaring_file, string_table);
            let diagnostic =
                CompilerDiagnostic::ambiguous_import_target(dependency_path.to_owned(), location);
            return Err(DependencyPathResolutionError::Diagnostic(Box::new(
                diagnostic,
            )));
        }

        let Some(candidate) = existing_candidates.first() else {
            let location = SourceLocation::from_path(declaring_file, string_table);
            return Err(DependencyPathResolutionError::Diagnostic(Box::new(
                CompilerDiagnostic::missing_import_target(dependency_path.clone(), location),
            )));
        };

        if candidate.support == DependencyCandidateSupport::RecognizedButUnsupported {
            let location = SourceLocation::from_path(declaring_file, string_table);
            let extension_id = string_table.intern(candidate.kind.extension());
            let diagnostic = CompilerDiagnostic::unsupported_source_file_kind(
                dependency_path.to_owned(),
                extension_id,
                location,
            );
            return Err(DependencyPathResolutionError::Diagnostic(Box::new(
                diagnostic,
            )));
        }

        let canonical = fs::canonicalize(&candidate.path).map_err(|error| {
            CompilerError::file_error(
                declaring_file,
                format!(
                    "Failed to canonicalize resolved dependency '{}': {error}",
                    dependency_path.to_portable_string(string_table)
                ),
                string_table,
            )
        })?;

        validate_dependency_boundary(
            &canonical,
            &base_kind,
            &filesystem_base,
            dependency_path,
            declaring_file,
            string_table,
        )?;
        validate_dependency_case_sensitivity(
            dependency_path,
            &base_kind,
            &filesystem_base,
            &canonical,
            candidate.is_parent_fallback,
            declaring_file,
            string_table,
        )?;

        let public_path = build_public_path(dependency_path, &base_kind, string_table);
        let ct_path = CompileTimePath {
            source_path: dependency_path.clone(),
            filesystem_path: canonical.clone(),
            public_path,
            base: base_kind,
        };
        Ok((ct_path, canonical))
    }

    /// WHAT: returns whether the dependency path starts with a registered source-backed package prefix.
    /// WHY: source-backed package dependencies should resolve to the package root, not fall through to entry root.
    fn matches_source_package_prefix(
        &self,
        dependency_path: &InternedPath,
        string_table: &StringTable,
    ) -> Option<PathBuf> {
        let first_component = dependency_path.as_components().first()?;
        let segment = string_table.resolve(*first_component);
        self.source_package_roots.roots().get(segment).cloned()
    }

    // -----------------------------------------------------------------------
    // Compile-time path literal resolution (non-dependency general paths)
    // -----------------------------------------------------------------------

    /// WHAT: resolves a general path literal to a typed compile-time path value.
    /// WHY: all Moth path literals must use the same resolution rules as
    ///       dependencies, but additionally require an existing regular file, reject
    ///       escapes outside the project root, and carry public-path metadata.
    pub(crate) fn resolve_compile_time_path(
        &self,
        path: &InternedPath,
        declaring_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<CompileTimePath, CompileTimePathResolutionError> {
        let (base_kind, filesystem_base) =
            self.resolve_path_base(path, declaring_file, string_table)?;

        let filesystem_path = join_and_normalize_path(&filesystem_base, path, string_table);

        self.validate_inside_project_root(&filesystem_path, path, declaring_file, string_table)?;

        validate_path_literal_target(&filesystem_path, path, declaring_file, string_table)?;

        let public_path = build_public_path(path, &base_kind, string_table);

        Ok(CompileTimePath {
            source_path: path.clone(),
            filesystem_path,
            public_path,
            base: base_kind,
        })
    }

    // -----------------------------------------------------------------------
    // Shared resolution helpers
    // -----------------------------------------------------------------------

    fn resolve_path_base(
        &self,
        path: &InternedPath,
        declaring_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<(CompileTimePathBase, PathBuf), CompilerError> {
        let declaring_dir = declaring_file.parent().ok_or_else(|| {
            CompilerError::file_error(
                declaring_file,
                "Could not determine parent directory for declaring file.",
                string_table,
            )
        })?;

        if is_relative_dependency_path(path, string_table) {
            Ok((
                CompileTimePathBase::RelativeToFile,
                declaring_dir.to_path_buf(),
            ))
        } else if let Some(package_root) = self.matches_source_package_prefix(path, string_table) {
            Ok((CompileTimePathBase::SourcePackageRoot, package_root))
        } else {
            Ok((CompileTimePathBase::EntryRoot, self.entry_root.clone()))
        }
    }

    fn source_kind_for_canonical_path(&self, path: &Path) -> Option<SourceFileKind> {
        let extension = path.extension().and_then(|extension| extension.to_str())?;
        SourceFileKind::from_extension(extension)
    }

    /// WHAT: rejects paths that would escape the project root after normalization.
    /// WHY: paths outside the project root are a semantic error in Moth.
    ///
    /// NOTE: `string_table` is only used on error paths to intern diagnostic file paths.
    fn validate_inside_project_root(
        &self,
        resolved: &Path,
        source_path: &InternedPath,
        declaring_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<(), CompileTimePathResolutionError> {
        // Canonicalize the project root once (it must exist).
        let canonical_root = fs::canonicalize(&self.project_root).map_err(|error| {
            CompilerError::file_error(
                &self.project_root,
                format!(
                    "Failed to canonicalize project root '{}': {error}",
                    self.project_root.display()
                ),
                string_table,
            )
        })?;

        // The resolved path may not exist yet (that check comes next), so we
        // walk up to the nearest existing ancestor and canonicalize from there,
        // then re-append the remaining tail.
        let canonical_resolved = canonicalize_best_effort(resolved);

        if !canonical_resolved.starts_with(&canonical_root) {
            let location = SourceLocation::from_path(declaring_file, string_table);
            let diagnostic = CompilerDiagnostic::invalid_compile_time_path(
                source_path.clone(),
                InvalidCompileTimePathReason::EscapesProjectRoot,
                location,
            );

            return Err(CompileTimePathResolutionError::Diagnostic(Box::new(
                diagnostic,
            )));
        }

        Ok(())
    }
}

fn explicit_source_extension(
    dependency_path: &InternedPath,
    string_table: &StringTable,
) -> Option<String> {
    for component in dependency_path.as_components() {
        let segment = string_table.resolve(*component);
        let Some(extension) = Path::new(segment)
            .extension()
            .and_then(|extension| extension.to_str())
        else {
            continue;
        };

        if SourceFileKind::from_extension(extension).is_some() {
            return Some(extension.to_owned());
        }
    }

    None
}

fn existing_dependency_candidates(candidates: &[DependencyCandidate]) -> Vec<&DependencyCandidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.path.is_file())
        .collect()
}

#[cfg(test)]
#[path = "tests/path_resolution_tests.rs"]
mod tests;

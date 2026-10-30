//! Project-aware dependency resolution.
//!
//! `ProjectPathResolver` owns Stage 0 source/dependency path rules and canonical source lookup.
//! Structural file values stay resolved physical targets here; their URL text is rendered later
//! by the consuming builder.
//!

use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidImportPathReason;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;

use crate::compiler_frontend::paths::dependency_resolution::{
    DependencyPathBase, DependencyPathResolutionError, validate_dependency_boundary,
    validate_dependency_case_sensitivity,
};
use crate::compiler_frontend::paths::path_normalization::{
    DependencyCandidate, DependencyCandidateSupport, candidate_dependency_files_for_source_kinds,
    dependency_contains_dotdot, is_relative_dependency_path, join_and_normalize_components,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
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
    /// WHAT: creates a resolver from canonical roots and Stage 0 module-root data.
    /// WHY: path resolution may query prepared module boundaries, but it must not perform
    ///      filesystem discovery during normal directory construction. Single-file compilation
    ///      passes `ModuleRootTable::empty()` for bare `.moth` entries
    ///      (`create_project_modules/compilation/single_file.rs:275-277`), so this constructor is
    ///      also the production entry point for the supported no-module-root state.
    pub(crate) fn new_with_module_roots(
        project_root: PathBuf,
        entry_root: PathBuf,
        source_package_roots: PreparedSourcePackageRoots,
        source_file_kinds: &SourceFileKindRegistry,
        module_roots: ModuleRootTable,
    ) -> Self {
        Self {
            project_root,
            entry_root,
            source_package_roots,
            module_roots,
            source_file_kinds: source_file_kinds.clone(),
        }
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
    /// WHAT: exposes the same whole-component package classification used by ordinary
    ///       dependency resolution to Stage 0's synthetic module-root precedence check.
    /// WHY: registered package paths must retain package-root semantics and must not be mistaken
    ///      for bare module-root-relative source paths.
    pub(crate) fn source_package_root_for_dependency(
        &self,
        dependency_path: PathId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Option<DependencyPathBase> {
        self.matches_source_package_prefix(dependency_path, path_fork, string_table)
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
    pub(crate) fn logical_path_for_canonical_file(
        &self,
        canonical_file: &Path,
    ) -> Result<PathBuf, InfrastructureFailure> {
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

        Err(InfrastructureFailure::filesystem(
            CompilerStage::DependencyResolution,
            canonical_file,
            format!(
                "Source file '{}' is outside both entry root '{}' and project root '{}'",
                canonical_file.display(),
                self.entry_root.display(),
                self.project_root.display()
            ),
        ))
    }

    /// WHAT: resolves a dependency path to a concrete source file and its source kind.
    /// WHY: Stage 0 must preserve the source kind so `.mtf` files can be discovered without being
    ///      scanned or prepared as normal Moth source.
    pub(crate) fn resolve_dependency_to_source_file(
        &self,
        dependency_path: PathId,
        path_fork: &PathInternerFork,
        declaring_file: &Path,
        string_table: &mut StringTable,
    ) -> Result<ResolvedDependencyFile, DependencyPathResolutionError> {
        let mut strings = DiagnosticStringBuilder::new(string_table);
        let (_, canonical) = self.resolve_dependency_as_compile_time_path(
            dependency_path,
            path_fork,
            declaring_file,
            &mut strings,
        )?;
        let source_kind = self
            .source_kind_for_canonical_path(&canonical)
            .ok_or_else(|| {
                InfrastructureFailure::filesystem(
                    CompilerStage::DependencyResolution,
                    declaring_file,
                    format!(
                        "Resolved dependency '{}' to '{}' but could not determine its source kind.",
                        path_fork.render_portable(
                            dependency_path,
                            strings.strings(),
                            &mut Vec::new(),
                        ),
                        canonical.display()
                    ),
                )
            })?;

        Ok(ResolvedDependencyFile {
            path: canonical,
            kind: source_kind,
        })
    }

    /// WHAT: resolves one dependency path to its semantic base and canonical file path.
    /// WHY: dependency resolution applies `.moth` extension fallback and boundary validation,
    ///      while structural file values consume the resulting Stage 0 facts separately.
    pub(crate) fn resolve_dependency_as_compile_time_path(
        &self,
        dependency_path: PathId,
        path_fork: &PathInternerFork,
        declaring_file: &Path,
        strings: &mut DiagnosticStringBuilder<'_>,
    ) -> Result<(CompileTimePathBase, PathBuf), DependencyPathResolutionError> {
        if let Some(extension) =
            explicit_source_extension(dependency_path, path_fork, strings.strings())
        {
            let diagnostic = if extension == SourceFileKind::Moth.extension() {
                imports::explicit_moth_extension(dependency_path.to_owned(), None)
            } else {
                let extension_id = strings.intern(&extension);
                imports::explicit_source_extension(dependency_path.to_owned(), extension_id, None)
            };
            return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
        }

        if dependency_contains_dotdot(dependency_path, path_fork, strings.strings()) {
            let diagnostic = imports::invalid_import_path(
                dependency_path.to_owned(),
                InvalidImportPathReason::ParentDirectorySegment,
                None,
            );
            return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
        }

        let base = self.resolve_path_base(
            dependency_path,
            path_fork,
            declaring_file,
            strings.strings(),
        )?;

        // A package root already represents its complete registered prefix.
        let mut components = Vec::new();
        let components = path_fork.resolve_components(dependency_path, &mut components);
        let normalized = join_and_normalize_components(
            &base.root,
            &components[base.matched_components..],
            strings.strings(),
        );

        // The matched package prefix counts as one logical component, so nested and single
        // component prefixes share the same parent-file fallback.
        let dependency_component_len = components.len() - base.matched_components.saturating_sub(1);
        let candidates = candidate_dependency_files_for_source_kinds(
            &normalized,
            dependency_component_len,
            self.source_file_kinds(),
        );
        let existing_candidates = existing_dependency_candidates(&candidates);
        let folder_exists = normalized.is_dir();

        if existing_candidates.len() + usize::from(folder_exists) > 1 {
            let diagnostic = imports::ambiguous_import_target(dependency_path.to_owned(), None);
            return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
        }

        let Some(candidate) = existing_candidates.first() else {
            return Err(DependencyPathResolutionError::Diagnostic(
                imports::missing_import_target(dependency_path, None),
            ));
        };

        if candidate.support == DependencyCandidateSupport::RecognizedButUnsupported {
            let extension_id = strings.intern(candidate.kind.extension());
            let diagnostic = imports::unsupported_source_file_kind(
                dependency_path.to_owned(),
                extension_id,
                None,
            );
            return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
        }

        let canonical = fs::canonicalize(&candidate.path).map_err(|error| {
            InfrastructureFailure::filesystem(
                CompilerStage::DependencyResolution,
                declaring_file,
                format!(
                    "Failed to canonicalize resolved dependency '{}': {error}",
                    path_fork.render_portable(dependency_path, strings.strings(), &mut Vec::new())
                ),
            )
        })?;

        validate_dependency_boundary(
            &canonical,
            &base.kind,
            &base.root,
            dependency_path,
            path_fork,
        )?;
        validate_dependency_case_sensitivity(
            dependency_path,
            path_fork,
            &base,
            &canonical,
            candidate.is_parent_fallback,
            strings,
        )?;

        Ok((base.kind, canonical))
    }

    /// Select the registered prefix that matches whole leading dependency components as a
    /// package-root base. Registration rejects overlapping prefixes, so at most one candidate
    /// matches.
    fn matches_source_package_prefix(
        &self,
        dependency_path: PathId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Option<DependencyPathBase> {
        let mut scratch = Vec::new();
        let mut prefix = path_fork.render_portable(dependency_path, string_table, &mut scratch);
        let mut depth = path_fork.depth(dependency_path) as usize;
        while depth > 0 {
            if let Some(root) = self.source_package_roots.roots().get(&prefix) {
                return Some(DependencyPathBase {
                    kind: CompileTimePathBase::SourcePackageRoot,
                    root: root.clone(),
                    matched_components: depth,
                });
            }
            let Some(separator) = prefix.rfind('/') else {
                break;
            };
            prefix.truncate(separator);
            depth -= 1;
        }
        None
    }
    // -----------------------------------------------------------------------
    // Shared resolution helpers
    // -----------------------------------------------------------------------

    fn resolve_path_base(
        &self,
        path: PathId,
        path_fork: &PathInternerFork,
        declaring_file: &Path,
        string_table: &StringTable,
    ) -> Result<DependencyPathBase, InfrastructureFailure> {
        let declaring_dir = declaring_file.parent().ok_or_else(|| {
            InfrastructureFailure::filesystem(
                CompilerStage::DependencyResolution,
                declaring_file,
                "Could not determine parent directory for declaring file.",
            )
        })?;
        if is_relative_dependency_path(path, path_fork, string_table) {
            return Ok(DependencyPathBase {
                kind: CompileTimePathBase::RelativeToFile,
                root: declaring_dir.to_path_buf(),
                matched_components: 0,
            });
        }
        if let Some(package_base) =
            self.matches_source_package_prefix(path, path_fork, string_table)
        {
            return Ok(package_base);
        }
        Ok(DependencyPathBase {
            kind: CompileTimePathBase::EntryRoot,
            root: self.entry_root.clone(),
            matched_components: 0,
        })
    }

    fn source_kind_for_canonical_path(&self, path: &Path) -> Option<SourceFileKind> {
        let extension = path.extension().and_then(|extension| extension.to_str())?;
        SourceFileKind::from_extension(extension)
    }
}
fn explicit_source_extension(
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Option<String> {
    let mut scratch = Vec::new();
    for component in path_fork.resolve_components(dependency_path, &mut scratch) {
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

//! Stage 0 physical resolution for graph-active file references.
//!
//! WHAT: resolves prepared, non-dependency file-reference rows from the consuming module root,
//! validates filesystem and module ownership, and publishes one compiler-facing outcome per
//! authored occurrence. Site-root and extensionless rows remain explicit no-target facts because
//! they have no Stage 0 physical target.
//! WHY: preparation owns shallow classification and AST owns value semantics. This build-system
//! owner is the only physical resolver for directory paths, so later stages receive settled
//! targets and cannot rediscover the filesystem. It never parses expressions or reads bytes.

use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCompileTimePathReason, PathKind,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReference, PreparedFileReferenceClass, ResolvedFileReference,
    ResolvedFileReferenceOutcome, ResolvedFileReferenceTarget, ResourceSourceId,
};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::source_packages::root_file::file_name_is_module_root_file;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::{FxHashMap, FxHashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::module_identity::ModuleId;
use super::resource_inputs::ResourceInputRegistry;
use super::source_tree_index::{SourceClassification, SourceId, SourceOwnership, SourceTreeIndex};

/// One directory-boundary physical file-reference resolver.
///
/// The source-tree index supplies module ownership and source classification; the resource
/// registry supplies the build-only physical source identity. The settled cache is scoped to one
/// module inventory and stores only physical outcomes, never semantic origins or bytes.
pub(crate) struct FileReferenceResolver<'a> {
    source_tree_index: &'a SourceTreeIndex,
    resource_inputs: &'a mut ResourceInputRegistry,
    containment_root: PathBuf,
    settled: FxHashMap<(PathBuf, PathBuf, PreparedFileReferenceClass), PhysicalResolution>,
}

#[derive(Clone, Debug)]
enum PhysicalResolution {
    Target(PathBuf),
    Missing {
        watch_path: PathBuf,
        canonical_ancestor: Option<PathBuf>,
    },
    Invalid(PhysicalInvalidReason),
}

#[derive(Clone, Debug)]
enum PhysicalInvalidReason {
    CaseMismatch { provided: String, expected: String },
    EscapesSymlink,
    TargetIsDirectory,
    TargetNotRegular,
}

enum LexicalCaseCheck {
    Clear,
    Mismatch { provided: String, expected: String },
}

impl<'a> FileReferenceResolver<'a> {
    pub(crate) fn new(
        source_tree_index: &'a SourceTreeIndex,
        resource_inputs: &'a mut ResourceInputRegistry,
    ) -> Self {
        Self {
            source_tree_index,
            resource_inputs,
            containment_root: source_tree_index.entry_root().to_path_buf(),
            settled: FxHashMap::default(),
        }
    }

    /// Resolve one graph-active occurrence from its consumer's module root.
    ///
    /// User-authored resolution failures become retained diagnostic outcomes. Missing targets
    /// also register a build-only watch interest. A disagreement between the source index and a
    /// canonical target is an infrastructure invariant failure and returns `CompilerError`.
    pub(crate) fn resolve(
        &mut self,
        consumer_module_id: ModuleId,
        path_syntax: &PathSyntaxTable,
        reference: &PreparedFileReference,
        source_files: &SourceFileTable,
        string_table: &mut StringTable,
        discovered_content_sources: &mut Vec<SourceId>,
    ) -> Result<ResolvedFileReference, CompilerError> {
        let source_file = reference.source_file.ok_or_else(|| {
            CompilerError::compiler_error("graph-active file reference has no preparing FileId")
        })?;
        let authored_path = &path_syntax
            .try_path_for_token(reference.path_syntax, &reference.location)?
            .root;

        match reference.class {
            PreparedFileReferenceClass::SiteRoot | PreparedFileReferenceClass::Extensionless => {
                return Ok(ResolvedFileReference {
                    source_file,
                    path_syntax: reference.path_syntax,
                    class: reference.class,
                    outcome: ResolvedFileReferenceOutcome::NoPhysicalTarget,
                });
            }
            PreparedFileReferenceClass::SourceKindNoFileValue => {
                return Ok(ResolvedFileReference {
                    source_file,
                    path_syntax: reference.path_syntax,
                    class: reference.class,
                    outcome: ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::IdentifiedSourceKind,
                    ),
                });
            }
            PreparedFileReferenceClass::ContentSource
            | PreparedFileReferenceClass::ResourceFile => {}
        }

        let root_directory = self
            .source_tree_index
            .module_identities()
            .record(consumer_module_id)
            .root_directory()
            .to_path_buf();
        let authored_components = authored_path
            .as_components()
            .iter()
            .map(|component| string_table.resolve(*component).to_owned())
            .collect::<Vec<_>>();

        if let Some(diagnostic) =
            invalid_components_diagnostic(&authored_components, authored_path, &reference.location)
        {
            return Ok(ResolvedFileReference {
                source_file,
                path_syntax: reference.path_syntax,
                class: reference.class,
                outcome: ResolvedFileReferenceOutcome::Diagnostic(Box::new(diagnostic)),
            });
        }

        let candidate = join_components(&root_directory, &authored_components);
        if self.lexically_escapes_module_boundary(consumer_module_id, &candidate) {
            return Ok(self.diagnostic_outcome(
                source_file,
                reference,
                authored_path,
                InvalidCompileTimePathReason::EscapesModuleBoundary,
            ));
        }
        let containment_root = self.containment_root.clone();
        let resolution = self.resolve_physical_target(
            &root_directory,
            &containment_root,
            &candidate,
            &authored_components,
            reference.class,
            string_table,
        )?;
        let canonical = match resolution {
            PhysicalResolution::Target(canonical) => canonical,
            PhysicalResolution::Missing {
                watch_path,
                canonical_ancestor,
            } => {
                let reason = canonical_ancestor.as_deref().is_some_and(|ancestor| {
                    self.canonical_ancestor_escapes_module_boundary(consumer_module_id, ancestor)
                });
                if reason {
                    return Ok(self.diagnostic_outcome(
                        source_file,
                        reference,
                        authored_path,
                        InvalidCompileTimePathReason::EscapesModuleBoundary,
                    ));
                }
                self.resource_inputs.record_missing_target_watch(watch_path);
                return Ok(self.diagnostic_outcome(
                    source_file,
                    reference,
                    authored_path,
                    InvalidCompileTimePathReason::MissingTarget,
                ));
            }
            PhysicalResolution::Invalid(reason) => {
                return Ok(self.diagnostic_outcome(
                    source_file,
                    reference,
                    authored_path,
                    invalid_reason(reason, string_table),
                ));
            }
        };

        let target_parent = canonical.parent().unwrap_or(&root_directory);
        if self
            .source_tree_index
            .module_identities()
            .nearest_module_for_directory(target_parent)
            != Some(consumer_module_id)
        {
            return Ok(self.diagnostic_outcome(
                source_file,
                reference,
                authored_path,
                InvalidCompileTimePathReason::EscapesModuleBoundary,
            ));
        }

        let outcome = match reference.class {
            PreparedFileReferenceClass::ContentSource => {
                let target_source_id = self
                    .source_tree_index
                    .source_id_for_canonical_path(&canonical)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "canonical content target {:?} is absent from SourceTreeIndex",
                            canonical
                        ))
                    })?;
                let target_record = self.source_tree_index.source(target_source_id);
                if !target_record.supported() {
                    let extension = canonical
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default();
                    return Ok(ResolvedFileReference {
                        source_file,
                        path_syntax: reference.path_syntax,
                        class: reference.class,
                        outcome: ResolvedFileReferenceOutcome::Diagnostic(Box::new(
                            CompilerDiagnostic::unsupported_source_file_kind(
                                authored_path.clone(),
                                string_table.intern(extension),
                                reference.location.clone(),
                            ),
                        )),
                    });
                }
                let target_source_id = self.indexed_source(
                    consumer_module_id,
                    &canonical,
                    SourceFileKind::from_extension(
                        canonical
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .unwrap_or_default(),
                    ),
                )?;
                discovered_content_sources.push(target_source_id);
                let target_file_id = source_files
                    .get_by_canonical_path(&canonical)
                    .map(|identity| identity.file_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "indexed content target is absent from the module SourceFileTable",
                        )
                    })?;
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
                    source: target_file_id,
                })
            }
            PreparedFileReferenceClass::ResourceFile => {
                let relative_path = candidate.strip_prefix(&root_directory).map_err(|_| {
                    CompilerError::compiler_error(
                        "resource candidate escaped its consumer module root after component validation",
                    )
                })?;
                let owner_relative_path =
                    PortableResourcePath::from_relative_logical_path(relative_path)?;
                let resource_source = self.resource_inputs.register_source(canonical.clone());
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
                    source: resource_source,
                    owner_relative_path,
                })
            }
            PreparedFileReferenceClass::SiteRoot
            | PreparedFileReferenceClass::Extensionless
            | PreparedFileReferenceClass::SourceKindNoFileValue => {
                unreachable!("non-physical file-reference class reached physical resolution")
            }
        };

        Ok(ResolvedFileReference {
            source_file,
            path_syntax: reference.path_syntax,
            class: reference.class,
            outcome,
        })
    }

    fn resolve_physical_target(
        &mut self,
        root_directory: &Path,
        containment_root: &Path,
        candidate: &Path,
        authored_components: &[String],
        class: PreparedFileReferenceClass,
        string_table: &mut StringTable,
    ) -> Result<PhysicalResolution, CompilerError> {
        resolve_physical_target_cached(
            &mut self.settled,
            root_directory,
            containment_root,
            candidate,
            authored_components,
            class,
            string_table,
        )
    }

    fn lexically_escapes_module_boundary(
        &self,
        consumer_module_id: ModuleId,
        candidate: &Path,
    ) -> bool {
        let Some(parent) = candidate.parent() else {
            return false;
        };
        self.source_tree_index
            .module_identities()
            .nearest_module_for_directory(parent)
            .is_some_and(|module_id| module_id != consumer_module_id)
    }

    fn canonical_ancestor_escapes_module_boundary(
        &self,
        consumer_module_id: ModuleId,
        canonical_ancestor: &Path,
    ) -> bool {
        self.source_tree_index
            .module_identities()
            .nearest_module_for_directory(canonical_ancestor)
            .is_some_and(|module_id| module_id != consumer_module_id)
    }

    fn indexed_source(
        &self,
        consumer_module_id: ModuleId,
        canonical: &Path,
        expected_kind: Option<SourceFileKind>,
    ) -> Result<SourceId, CompilerError> {
        let source_id = self
            .source_tree_index
            .source_id_for_canonical_path(canonical)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "canonical source target {:?} is absent from SourceTreeIndex",
                    canonical
                ))
            })?;
        let record = self.source_tree_index.source(source_id);
        if record.ownership() != SourceOwnership::Owned(consumer_module_id) {
            return Err(CompilerError::compiler_error(format!(
                "canonical source target {:?} disagrees with indexed ownership facts",
                canonical
            )));
        }
        if !record.supported() {
            return Err(CompilerError::compiler_error(format!(
                "unsupported source target {:?} must be diagnosed before indexed_source",
                canonical
            )));
        }

        let SourceClassification::CompilerSemantic(actual_kind) = record.classification() else {
            return Err(CompilerError::compiler_error(format!(
                "canonical source target {:?} is not compiler semantic",
                canonical
            )));
        };
        if expected_kind != Some(*actual_kind) {
            return Err(CompilerError::compiler_error(format!(
                "canonical source target {:?} has source kind {:?}, expected {:?}",
                canonical, actual_kind, expected_kind
            )));
        }

        Ok(source_id)
    }

    fn diagnostic_outcome(
        &self,
        source_file: FileId,
        reference: &PreparedFileReference,
        authored_path: &InternedPath,
        reason: InvalidCompileTimePathReason,
    ) -> ResolvedFileReference {
        ResolvedFileReference {
            source_file,
            path_syntax: reference.path_syntax,
            class: reference.class,
            outcome: ResolvedFileReferenceOutcome::Diagnostic(Box::new(
                CompilerDiagnostic::invalid_compile_time_path(
                    authored_path.clone(),
                    reason,
                    reference.location.clone(),
                ),
            )),
        }
    }
}

fn resolve_physical_target_cached(
    settled: &mut FxHashMap<(PathBuf, PathBuf, PreparedFileReferenceClass), PhysicalResolution>,
    root_directory: &Path,
    containment_root: &Path,
    candidate: &Path,
    authored_components: &[String],
    class: PreparedFileReferenceClass,
    string_table: &mut StringTable,
) -> Result<PhysicalResolution, CompilerError> {
    let key = (root_directory.to_path_buf(), candidate.to_path_buf(), class);
    if let Some(resolution) = settled.get(&key) {
        return Ok(resolution.clone());
    }

    let resolution = match lexical_case_check(root_directory, authored_components, string_table)? {
        LexicalCaseCheck::Mismatch { provided, expected } => {
            PhysicalResolution::Invalid(PhysicalInvalidReason::CaseMismatch { provided, expected })
        }
        LexicalCaseCheck::Clear => match fs::canonicalize(candidate) {
            Ok(canonical) => {
                if !canonical.starts_with(containment_root) {
                    PhysicalResolution::Invalid(PhysicalInvalidReason::EscapesSymlink)
                } else {
                    let metadata = fs::metadata(&canonical).map_err(|error| {
                        CompilerError::file_error(
                            &canonical,
                            format!("Failed to inspect file-value target: {error}"),
                            string_table,
                        )
                    })?;
                    if metadata.is_dir() {
                        PhysicalResolution::Invalid(PhysicalInvalidReason::TargetIsDirectory)
                    } else if !metadata.is_file() {
                        PhysicalResolution::Invalid(PhysicalInvalidReason::TargetNotRegular)
                    } else {
                        PhysicalResolution::Target(canonical)
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let missing = resolve_missing_target_evidence(candidate, string_table)?;
                if missing.non_directory_ancestor {
                    PhysicalResolution::Invalid(PhysicalInvalidReason::TargetNotRegular)
                } else if missing
                    .canonical_ancestor
                    .as_deref()
                    .is_some_and(|ancestor| !ancestor.starts_with(containment_root))
                {
                    PhysicalResolution::Invalid(PhysicalInvalidReason::EscapesSymlink)
                } else {
                    PhysicalResolution::Missing {
                        watch_path: missing.watch_path,
                        canonical_ancestor: missing.canonical_ancestor,
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotADirectory => {
                PhysicalResolution::Invalid(PhysicalInvalidReason::TargetNotRegular)
            }
            Err(error) => {
                return Err(CompilerError::file_error(
                    candidate,
                    format!("Failed to resolve file-value target: {error}"),
                    string_table,
                ));
            }
        },
    };

    settled.insert(key, resolution.clone());
    Ok(resolution)
}

struct MissingTargetEvidence {
    watch_path: PathBuf,
    canonical_ancestor: Option<PathBuf>,
    non_directory_ancestor: bool,
}

/// Resolve a missing target component by component, following dangling symlinks lexically so
/// ownership and containment are settled before a watch is published. Symlink target components
/// are spliced ahead of the remaining path and only then interpreted, preserving `link/..`
/// semantics. The evidence is retained in the physical cache for repeated occurrences.
fn resolve_missing_target_evidence(
    candidate: &Path,
    string_table: &mut StringTable,
) -> Result<MissingTargetEvidence, CompilerError> {
    const MAX_SYMLINK_FOLLOWS: usize = 40;

    let mut pending = candidate.to_path_buf();
    let mut seen_paths = FxHashSet::default();
    seen_paths.insert(pending.clone());
    let mut symlinks_followed = 0;

    loop {
        let mut current = PathBuf::new();
        let mut components = pending.components().peekable();
        let mut replaced_symlink = None;

        while let Some(component) = components.next() {
            use std::path::Component;

            match component {
                Component::Prefix(prefix) => current.push(prefix.as_os_str()),
                Component::RootDir => current.push(component.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => {
                    current.pop();
                }
                Component::Normal(name) => {
                    let next = current.join(name);
                    match fs::symlink_metadata(&next) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            let target = fs::read_link(&next).map_err(|error| {
                                CompilerError::file_error(
                                    &next,
                                    format!(
                                        "Failed to read dangling file-value symlink target: {error}"
                                    ),
                                    string_table,
                                )
                            })?;
                            let mut replacement = if target.is_absolute() {
                                target
                            } else {
                                next.parent().unwrap_or_else(|| Path::new("")).join(target)
                            };
                            for remaining in components {
                                replacement.push(remaining.as_os_str());
                            }
                            symlinks_followed += 1;
                            if symlinks_followed > MAX_SYMLINK_FOLLOWS
                                || !seen_paths.insert(replacement.clone())
                            {
                                return Err(CompilerError::file_error(
                                    &next,
                                    "File-value symlink chain contains a cycle or exceeds the supported depth",
                                    string_table,
                                ));
                            }
                            replaced_symlink = Some(replacement);
                            break;
                        }
                        Ok(_) => current.push(name),
                        Err(error)
                            if matches!(
                                error.kind(),
                                ErrorKind::NotFound | ErrorKind::NotADirectory
                            ) =>
                        {
                            return Ok(MissingTargetEvidence {
                                watch_path: next,
                                non_directory_ancestor: fs::symlink_metadata(&current)
                                    .map(|metadata| !metadata.is_dir())
                                    .unwrap_or(false),
                                canonical_ancestor: canonicalize_existing_path(
                                    &current,
                                    string_table,
                                )?,
                            });
                        }
                        Err(error) => {
                            return Err(CompilerError::file_error(
                                &next,
                                format!(
                                    "Failed to inspect missing file-value path component: {error}"
                                ),
                                string_table,
                            ));
                        }
                    }
                }
            }
        }

        if let Some(replacement) = replaced_symlink {
            pending = replacement;
            continue;
        }

        return Ok(MissingTargetEvidence {
            watch_path: pending.clone(),
            canonical_ancestor: canonicalize_existing_path(&pending, string_table)?,
            non_directory_ancestor: false,
        });
    }
}

/// Canonicalize a physical path that was already proven to be the longest existing prefix.
fn canonicalize_existing_path(
    existing_path: &Path,
    string_table: &mut StringTable,
) -> Result<Option<PathBuf>, CompilerError> {
    match fs::canonicalize(existing_path) {
        Ok(canonical) => Ok(Some(canonical)),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(None)
        }
        Err(error) => Err(CompilerError::file_error(
            existing_path,
            format!("Failed to resolve missing file-value ancestor: {error}"),
            string_table,
        )),
    }
}

/// Join validated authored components without normalizing away their lexical spelling.
fn join_components(root: &Path, components: &[String]) -> PathBuf {
    let mut candidate = root.to_path_buf();
    for component in components {
        candidate.push(component);
    }
    candidate
}

/// Check authored lexical spelling before canonicalization follows any symlink.
fn lexical_case_check(
    root_directory: &Path,
    components: &[String],
    string_table: &mut StringTable,
) -> Result<LexicalCaseCheck, CompilerError> {
    let mut current = root_directory.to_path_buf();
    for component in components {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                return Ok(LexicalCaseCheck::Clear);
            }
            Err(error) => {
                return Err(CompilerError::file_error(
                    &current,
                    format!("Failed to inspect file-value path casing: {error}"),
                    string_table,
                ));
            }
        };

        let mut exact_match = false;
        let folded_component = component.to_lowercase();
        let mut case_insensitive_matches = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                CompilerError::file_error(
                    &current,
                    format!("Failed to inspect file-value directory entry: {error}"),
                    string_table,
                )
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name == component {
                exact_match = true;
                break;
            }
            if name.to_lowercase() == folded_component {
                case_insensitive_matches.push(name.to_owned());
            }
        }

        if exact_match {
            current.push(component);
        } else if let Some(expected) =
            stable_case_mismatch(component, &mut case_insensitive_matches)
        {
            return Ok(LexicalCaseCheck::Mismatch {
                provided: component.to_owned(),
                expected,
            });
        } else {
            // The canonicalization step distinguishes a missing component from other IO errors.
            current.push(component);
        }
    }

    Ok(LexicalCaseCheck::Clear)
}

/// Select deterministic evidence for a strict-spelling mismatch.
///
/// Filesystems may expose more than one spelling that compares equal under Unicode lowercasing.
/// The diagnostic model has no ambiguity variant, so stable lexical ordering keeps the retained
/// evidence reproducible without allowing directory iteration order to leak into diagnostics.
pub(super) fn stable_case_mismatch(component: &str, matches: &mut Vec<String>) -> Option<String> {
    let folded_component = component.to_lowercase();
    matches.retain(|candidate| candidate.to_lowercase() == folded_component);
    matches.sort_unstable();
    matches.drain(..).next()
}

fn invalid_reason(
    reason: PhysicalInvalidReason,
    string_table: &mut StringTable,
) -> InvalidCompileTimePathReason {
    match reason {
        PhysicalInvalidReason::CaseMismatch { provided, expected } => {
            InvalidCompileTimePathReason::CaseMismatch {
                provided: string_table.intern(&provided),
                expected: string_table.intern(&expected),
            }
        }
        PhysicalInvalidReason::EscapesSymlink => InvalidCompileTimePathReason::EscapesSymlink,
        PhysicalInvalidReason::TargetIsDirectory => InvalidCompileTimePathReason::TargetIsDirectory,
        PhysicalInvalidReason::TargetNotRegular => InvalidCompileTimePathReason::TargetNotRegular,
    }
}

/// Reject authored components that can never be a physical path under the module root.
///
/// Leading `@` is deliberately absent here: it is a parser concern — unquoted `@@` is already
/// rejected by the parser, and quoted `"@logo.svg"` is a legal filesystem component.
fn invalid_components_diagnostic(
    components: &[String],
    authored_path: &InternedPath,
    location: &SourceLocation,
) -> Option<CompilerDiagnostic> {
    for component in components {
        if component == "." {
            return Some(CompilerDiagnostic::invalid_compile_time_path(
                authored_path.clone(),
                InvalidCompileTimePathReason::CurrentDirectorySegment,
                location.clone(),
            ));
        }
        if component == ".." {
            return Some(CompilerDiagnostic::invalid_compile_time_path(
                authored_path.clone(),
                InvalidCompileTimePathReason::ParentDirectorySegment,
                location.clone(),
            ));
        }
        let path = Path::new(component);
        if path.is_absolute() {
            return Some(CompilerDiagnostic::invalid_path(
                PathKind::InvalidComponent,
                location.clone(),
            ));
        }
    }

    None
}

/// One retained invalid-path diagnostic for a synthesized single-file reference.
fn invalid_path_outcome(
    authored_path: &InternedPath,
    reason: InvalidCompileTimePathReason,
    location: &SourceLocation,
) -> SingleFileReferenceOutcome {
    SingleFileReferenceOutcome::Diagnostic(Box::new(CompilerDiagnostic::invalid_compile_time_path(
        authored_path.clone(),
        reason,
        location.clone(),
    )))
}

/// A Stage 0 outcome retained while synthetic single-file discovery is still assembling its
/// source closure. The final `FileId` values are assigned only after that closure is complete.
#[derive(Clone, Debug)]
pub(crate) struct SingleFileResolvedReference {
    pub(crate) source_path: PathBuf,
    pub(crate) path_syntax: PathSyntaxId,
    pub(crate) class: PreparedFileReferenceClass,
    pub(crate) outcome: SingleFileReferenceOutcome,
}

#[derive(Clone, Debug)]
pub(crate) enum SingleFileReferenceOutcome {
    NoPhysicalTarget,
    Source {
        canonical: PathBuf,
    },
    IdentifiedSourceKind,
    Resource {
        source: ResourceSourceId,
        owner_relative_path: PortableResourcePath,
    },
    Diagnostic(Box<CompilerDiagnostic>),
}

/// Physical resolver for synthetic single-file discovery.
///
/// It shares the exact same lexical validation, canonical containment and settled cache as the
/// directory resolver. The synthetic mode has no indexed `ModuleId`, so source ownership is
/// represented by its containing directory and the final module preparation maps canonical source
/// paths to its deterministic `FileId`s. Module-boundary legality for file values is detected
/// lazily: every directory between a target and the synthetic root is probed with one `read_dir`
/// for a child module root file, with the probe verdict cached per directory.
pub(crate) struct SingleFileReferenceResolver<'a> {
    root_directory: PathBuf,
    containment_root: PathBuf,
    boundary_cache: FxHashMap<PathBuf, bool>,
    source_file_kinds: &'a SourceFileKindRegistry,
    resource_inputs: &'a mut ResourceInputRegistry,
    settled: FxHashMap<(PathBuf, PathBuf, PreparedFileReferenceClass), PhysicalResolution>,
}

impl<'a> SingleFileReferenceResolver<'a> {
    pub(crate) fn new(
        root_directory: PathBuf,
        source_file_kinds: &'a SourceFileKindRegistry,
        resource_inputs: &'a mut ResourceInputRegistry,
    ) -> Self {
        let containment_root = root_directory.clone();
        Self {
            root_directory,
            containment_root,
            boundary_cache: FxHashMap::default(),
            source_file_kinds,
            resource_inputs,
            settled: FxHashMap::default(),
        }
    }

    pub(crate) fn resolve(
        &mut self,
        source_path: &Path,
        path_syntax: &PathSyntaxTable,
        reference: &PreparedFileReference,
        string_table: &mut StringTable,
    ) -> Result<SingleFileResolvedReference, CompilerError> {
        let authored_path = &path_syntax
            .try_path_for_token(reference.path_syntax, &reference.location)?
            .root;
        let result = SingleFileResolvedReference {
            source_path: source_path.to_path_buf(),
            path_syntax: reference.path_syntax,
            class: reference.class,
            outcome: SingleFileReferenceOutcome::NoPhysicalTarget,
        };
        match reference.class {
            PreparedFileReferenceClass::SiteRoot | PreparedFileReferenceClass::Extensionless => {
                return Ok(result);
            }
            PreparedFileReferenceClass::SourceKindNoFileValue => {
                return Ok(SingleFileResolvedReference {
                    outcome: SingleFileReferenceOutcome::IdentifiedSourceKind,
                    ..result
                });
            }
            PreparedFileReferenceClass::ContentSource
            | PreparedFileReferenceClass::ResourceFile => {}
        }

        let authored_components = authored_path
            .as_components()
            .iter()
            .map(|component| string_table.resolve(*component).to_owned())
            .collect::<Vec<_>>();
        if let Some(diagnostic) =
            invalid_components_diagnostic(&authored_components, authored_path, &reference.location)
        {
            return Ok(SingleFileResolvedReference {
                outcome: SingleFileReferenceOutcome::Diagnostic(Box::new(diagnostic)),
                ..result
            });
        }

        let candidate = join_components(&self.root_directory, &authored_components);
        let lexically_escapes = match candidate.parent() {
            Some(parent) => self.chain_escapes_module_boundary(parent, string_table)?,
            None => false,
        };
        if lexically_escapes {
            return Ok(SingleFileResolvedReference {
                outcome: invalid_path_outcome(
                    authored_path,
                    InvalidCompileTimePathReason::EscapesModuleBoundary,
                    &reference.location,
                ),
                ..result
            });
        }

        let resolution = resolve_physical_target_cached(
            &mut self.settled,
            &self.root_directory,
            &self.containment_root,
            &candidate,
            &authored_components,
            reference.class,
            string_table,
        )?;
        let canonical = match resolution {
            PhysicalResolution::Target(canonical) => canonical,
            PhysicalResolution::Missing {
                watch_path,
                canonical_ancestor,
            } => {
                let escapes = match canonical_ancestor.as_deref() {
                    Some(ancestor) => self.chain_escapes_module_boundary(ancestor, string_table)?,
                    None => false,
                };
                if escapes {
                    return Ok(SingleFileResolvedReference {
                        outcome: invalid_path_outcome(
                            authored_path,
                            InvalidCompileTimePathReason::EscapesModuleBoundary,
                            &reference.location,
                        ),
                        ..result
                    });
                }
                self.resource_inputs.record_missing_target_watch(watch_path);
                return Ok(SingleFileResolvedReference {
                    outcome: invalid_path_outcome(
                        authored_path,
                        InvalidCompileTimePathReason::MissingTarget,
                        &reference.location,
                    ),
                    ..result
                });
            }
            PhysicalResolution::Invalid(reason) => {
                return Ok(SingleFileResolvedReference {
                    outcome: invalid_path_outcome(
                        authored_path,
                        invalid_reason(reason, string_table),
                        &reference.location,
                    ),
                    ..result
                });
            }
        };

        if self.chain_escapes_module_boundary(&canonical, string_table)? {
            return Ok(SingleFileResolvedReference {
                outcome: invalid_path_outcome(
                    authored_path,
                    InvalidCompileTimePathReason::EscapesModuleBoundary,
                    &reference.location,
                ),
                ..result
            });
        }

        if matches!(reference.class, PreparedFileReferenceClass::ContentSource) {
            let extension = canonical
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            if !self
                .source_file_kinds
                .supports_recognized_extension(extension)
            {
                return Ok(SingleFileResolvedReference {
                    outcome: SingleFileReferenceOutcome::Diagnostic(Box::new(
                        CompilerDiagnostic::unsupported_source_file_kind(
                            authored_path.clone(),
                            string_table.intern(extension),
                            reference.location.clone(),
                        ),
                    )),
                    ..result
                });
            }
            return Ok(SingleFileResolvedReference {
                outcome: SingleFileReferenceOutcome::Source { canonical },
                ..result
            });
        }

        let owner_relative_path = candidate.strip_prefix(&self.root_directory).map_err(|_| {
            CompilerError::compiler_error(
                "single-file resource candidate escaped its module root after validation",
            )
        })?;
        let owner_relative_path =
            PortableResourcePath::from_relative_logical_path(owner_relative_path)?;
        let source = self.resource_inputs.register_source(canonical);
        Ok(SingleFileResolvedReference {
            outcome: SingleFileReferenceOutcome::Resource {
                source,
                owner_relative_path,
            },
            ..result
        })
    }

    /// Walk the directory chain from `path` up to (and excluding) the synthetic module root and
    /// report whether any directory on that chain is the root of another normal or support module.
    /// Each probe is one `read_dir` cached per directory, so repeated occurrences never re-read.
    fn chain_escapes_module_boundary(
        &mut self,
        path: &Path,
        string_table: &mut StringTable,
    ) -> Result<bool, CompilerError> {
        for directory in path.ancestors() {
            if directory == self.root_directory {
                break;
            }
            if !directory.starts_with(&self.root_directory) {
                break;
            }
            if self.directory_is_module_root_boundary(directory, string_table)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Report whether `directory` directly holds a module root file, caching the `read_dir`
    /// verdict. Missing and non-directory paths are not boundaries; other read failures are
    /// infrastructure errors.
    fn directory_is_module_root_boundary(
        &mut self,
        directory: &Path,
        string_table: &mut StringTable,
    ) -> Result<bool, CompilerError> {
        if let Some(is_boundary) = self.boundary_cache.get(directory) {
            return Ok(*is_boundary);
        }
        let is_boundary = match fs::read_dir(directory) {
            Ok(entries) => {
                let mut is_boundary = false;
                for entry in entries {
                    let entry = entry.map_err(|error| {
                        CompilerError::file_error(
                            directory,
                            format!("Failed to read module boundary directory entry: {error}"),
                            string_table,
                        )
                    })?;
                    let entry_path = entry.path();
                    if entry_path.is_file()
                        && entry_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(file_name_is_module_root_file)
                    {
                        is_boundary = true;
                        break;
                    }
                }
                is_boundary
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                false
            }
            Err(error) => {
                return Err(CompilerError::file_error(
                    directory,
                    format!("Failed to inspect module boundary directory: {error}"),
                    string_table,
                ));
            }
        };
        self.boundary_cache
            .insert(directory.to_path_buf(), is_boundary);
        Ok(is_boundary)
    }
}

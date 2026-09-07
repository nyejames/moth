//! Stage 0 bundle construction for one direct Moth template compilation.
//!
//! WHAT: walks one template's content-source fixed point with the build-owned physical resolver,
//!       collects every nested `.mtf`/`.md` dependency, assigns source identities in canonical
//!       logical order, rebinds the one discovery-prepared output per source onto those identities
//!       and assembles the compiler service's file-value bundle.
//! WHY:  physical file-reference resolution stays build-owned. The compiler service folds settled
//!       Stage 0 facts and never probes the filesystem, while watch and invalidation policy stays
//!       with this direct API's callers.
//!
//! The fixed point mirrors synthetic single-file discovery: content targets resolve relative to
//! the compiling template's directory (this lane's entry root), the walk collects the canonical
//! candidate set in BFS order, identities are assigned once from that set, and every prepared
//! occurrence keeps its settled outcome. Route and output placement are never built here.

use crate::build_system::create_project_modules::extract_source_code;
use crate::build_system::create_project_modules::file_reference_resolution::{
    SingleFileReferenceOutcome, SingleFileReferenceResolver, SingleFileResolvedReference,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderParseOptions,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::single_source_compilation::MothTemplateFileValueBundle;
use crate::compiler_frontend::source::{
    SourceDatabase, SourceId, SourceKind, SourceRegistrationIndex,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::projects::html_project::moth_template::input::MothTemplateSourceUnit;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The project-local package identity of one direct-template compilation request.
///
/// Direct template compilation has no configured project, so the module origin and the render
/// plan agree on this name. Matching names keep resource placement bare: a resource's output
/// path is its path relative to the document's portable directory under the request's shared
/// module identity.
pub(super) const DIRECT_TEMPLATE_PROJECT_NAME: &str = "moth-template";

/// One file the content fixed point has not prepared yet.
enum QueuedSource {
    /// The compiling template itself; its text is already in the source unit.
    Entry,
    /// A content dependency read from disk when reached.
    Content { path: PathBuf, kind: SourceFileKind },
}

/// Prepare one direct template's file-value bundle and register its physical facts.
///
/// The template's own directory is the entry root: references resolve module-root-relative to it
/// and cannot reach another module. The registry is supplied by the request's owner so issued
/// resource sources and their origin attachments outlive this one document; this lane never
/// creates a private one for it to be dropped from.
pub(super) fn prepare_file_value_bundle(
    unit: &mut MothTemplateSourceUnit,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
    resource_inputs: &mut ResourceInputRegistry,
) -> Result<MothTemplateFileValueBundle, CompilerMessages> {
    let module_root = unit
        .source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let source_file_kinds = recognised_source_file_kinds();
    let path_resolver = ProjectPathResolver::new(
        module_root.clone(),
        module_root.clone(),
        PreparedSourcePackageRoots::empty(),
        &source_file_kinds,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // Direct-template resources belong to the module origin the document's portable relative
    // directory names: sibling documents in distinct directories mint distinct origins, while a
    // single-file request keeps the entry-root empty module path. Sharing the project name with
    // the render plan keeps resource placement relative to the template's directory.
    let module_relative_path = unit
        .relative_path
        .as_deref()
        .unwrap_or_else(|| Path::new(""));
    let module_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local(DIRECT_TEMPLATE_PROJECT_NAME),
        module_relative_path
            .parent()
            .unwrap_or_else(|| Path::new("")),
        ModuleRootRole::Normal,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // 1. Walk the content fixed point with traversal-local identities. Nested `.mtf`/`.md`
    //    targets join the candidate set in BFS order; final identities wait until that set is
    //    complete.
    let mut discovery_files = SourceDatabase::empty();
    // The request owner's registry receives every source this resolver issues. Watch and
    // missing-target interests remain this lane's physical facts for the caller to use.
    let mut resolver =
        SingleFileReferenceResolver::new(module_root.clone(), &source_file_kinds, resource_inputs);

    // The entry is registered before the walk so header preparation can name the entry file by
    // identity. Its first BFS visit re-registers the same path and kind, returning this same ID.
    let entry_file_id = discovery_files
        .insert(
            unit.source_path.clone(),
            SourceKind::Compiler(SourceFileKind::MothTemplate),
            &unit.source_path,
            Some(&path_resolver),
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let discovery_options = HeaderParseOptions {
        entry_file_id: Some(entry_file_id),
        project_path_resolver: Some(&path_resolver),
        entry_file_role: None,
        active_root_role: ModuleRootRole::Normal,
    };

    let mut candidates: FxHashMap<PathBuf, SourceFileKind> = FxHashMap::default();
    candidates.insert(unit.source_path.clone(), SourceFileKind::MothTemplate);

    let mut loaded: FxHashMap<PathBuf, Result<String, CompilerError>> = FxHashMap::default();
    loaded.insert(
        unit.source_path.clone(),
        Ok(std::mem::take(&mut unit.source_text)),
    );
    let mut queue = VecDeque::new();
    queue.push_back(QueuedSource::Entry);
    // One row per visited source, in discovery order. Pairing the path with its own prepared
    // output here keeps the identity rebinding below from depending on two vectors staying
    // the same length.
    let mut prepared_sources: Vec<(PathBuf, FileFrontendPrepareOutput)> = Vec::new();
    let mut pending_references = Vec::new();
    let mut visited = FxHashSet::default();

    while let Some(pending) = queue.pop_front() {
        let (path, kind) = match pending {
            QueuedSource::Entry => (unit.source_path.clone(), SourceFileKind::MothTemplate),
            QueuedSource::Content { path, kind } => (path, kind),
        };
        if !visited.insert(path.clone()) {
            continue;
        }

        let prepared = {
            let source_code = match loaded.get(&path) {
                Some(Ok(source_code)) => source_code.as_str(),
                Some(Err(error)) => {
                    // Loading moved earlier than preparation, so a recorded read failure is
                    // reported here instead: this is the point at which the read failed before.
                    return Err(CompilerMessages::from_error_ref(
                        error.clone(),
                        string_table,
                    ));
                }
                None => {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "content source {path:?} has no retained source text"
                        )),
                        string_table,
                    ));
                }
            };

            // Register immediately before preparation so every token stream receives a
            // traversal-local identity.
            discovery_files
                .insert(
                    path.clone(),
                    SourceKind::Compiler(kind),
                    &unit.source_path,
                    Some(&path_resolver),
                    string_table,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

            prepare_one_source(
                &discovery_files,
                &path,
                kind,
                source_code,
                &discovery_options,
                style_directives,
                string_table,
            )?
        };
        let path_syntax_table = prepared.path_syntax.table();
        let structural_references = prepared.structural_file_references.references();

        for reference in structural_references {
            let resolved = resolver
                .resolve(&path, path_syntax_table, reference, string_table)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            collect_content_candidate(
                &resolved,
                &mut candidates,
                &mut loaded,
                &mut queue,
                &source_file_kinds,
                string_table,
            )?;
            pending_references.push(resolved);
        }
        prepared_sources.push((path, prepared));
    }

    // 2. Assign identities from the complete candidate set in canonical logical order, rebind
    //    each discovery-prepared output onto those finished SourceIds, and join physical
    //    outcomes onto the classified table.
    let mut source_files = {
        let classified_rows = candidates
            .into_iter()
            .map(|(path, kind)| (path, SourceKind::Compiler(kind)))
            .collect::<Vec<_>>();
        let registration_index = SourceRegistrationIndex::from_rows(
            classified_rows
                .iter()
                .map(|(path, kind)| (path.as_path(), *kind)),
        );
        // The database copies each canonical path into its record, so the classified rows and the
        // index they back are dead once registration returns.
        SourceDatabase::from_registration_index_sorted_by_logical_path(
            &registration_index,
            &unit.source_path,
            Some(&path_resolver),
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
    };

    for (path, snapshot) in loaded {
        let source_id = source_id_for_path(
            &source_files,
            &path,
            string_table,
            "has no identity after classified registration",
        )?;
        match snapshot {
            Ok(source_code) => source_files
                .retain_text(source_id, source_code)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
            Err(_) => {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "content source {path:?} kept a read failure past preparation"
                    )),
                    string_table,
                ));
            }
        }
    }

    let mut prepared_content_sources = Vec::new();
    for (path, mut prepared) in prepared_sources {
        if path == unit.source_path {
            continue;
        }
        let record = source_files.get_by_canonical_path(&path).ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "source {path:?} has no identity before rebinding"
                )),
                string_table,
            )
        })?;
        let canonical_os_path = record.canonical_os_path.clone().ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "final source identity {} has no canonical path",
                    record.id.index()
                )),
                string_table,
            )
        })?;
        prepared
            .rebind_source_identity(
                record.id,
                source_files.legacy_logical_path(record.id),
                canonical_os_path,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        prepared
            .freeze_path_syntax(string_table)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        prepared_content_sources.push(prepared);
    }

    let mut resolved_file_references = ResolvedFileReferenceTable::new();
    for resolved in pending_references {
        // Discovery prepared this reference's owner with a traversal-local SourceId. The shared
        // path resolver computes the same logical path during final registration, but final
        // sorted identities can differ. Prepared outputs were rebound above. The diagnostic is
        // separately owned, so keep its identity normalization alongside the final row's SourceId
        // assignment even though the logical path is already final.
        let owner = source_files
            .get_by_canonical_path(&resolved.source_path)
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "source {:?} owner has no classified source identity",
                        resolved.source_path
                    )),
                    string_table,
                )
            })?;
        let owner_source_file = owner.id;
        let owner_logical_path = source_files.legacy_logical_path(owner_source_file);
        let path_syntax = resolved.path_syntax;
        let class = resolved.class;
        let mut outcome = resolved_outcome_from_physical(resolved, &source_files, string_table)?;
        if let ResolvedFileReferenceOutcome::Diagnostic(diagnostic) = &mut outcome {
            diagnostic.rebind_source_identity(&owner_logical_path);
        }
        resolved_file_references
            .push(ResolvedFileReference {
                source_file: owner_source_file,
                path_syntax,
                class,
                outcome,
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    Ok(MothTemplateFileValueBundle {
        prepared_content_sources,
        resolved_file_references,
        source_files: Arc::new(source_files),
        module_origin: Some(module_origin),
    })
}

/// Record one physically resolved content target in the candidate set and queue it.
///
/// Loading happens here, before the caller's visited check, so it must dedupe on the loaded
/// snapshot rather than on the queue. A path already collected keeps the kind its own producer
/// authored. Only a genuinely new path is classified here, because a lexical spelling can resolve
/// onto a target whose extension names another kind, and asserting a second kind for one
/// canonical path is the conflict the source database rejects.
fn collect_content_candidate(
    resolved: &SingleFileResolvedReference,
    candidates: &mut FxHashMap<PathBuf, SourceFileKind>,
    loaded: &mut FxHashMap<PathBuf, Result<String, CompilerError>>,
    queue: &mut VecDeque<QueuedSource>,
    source_file_kinds: &SourceFileKindRegistry,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    match &resolved.outcome {
        SingleFileReferenceOutcome::IdentifiedSourceKind => {
            if resolved.class != PreparedFileReferenceClass::SourceKindNoFileValue {
                return Err(incompatible_class_messages(string_table));
            }
            Ok(())
        }
        SingleFileReferenceOutcome::Source { canonical } => {
            if resolved.class != PreparedFileReferenceClass::ContentSource {
                return Err(incompatible_class_messages(string_table));
            }

            let kind = if let Some(&kind) = candidates.get(canonical) {
                kind
            } else {
                let extension = canonical
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default();
                let Some(kind) = source_file_kinds.kind_for_extension(extension) else {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "resolved supported content target has no registered source kind",
                        ),
                        string_table,
                    ));
                };
                candidates.insert(canonical.clone(), kind);
                kind
            };

            if !loaded.contains_key(canonical) {
                // A read failure is recorded against the snapshot rather than returned here. The
                // reference is still queued so the failure surfaces when this source is popped
                // for preparation, which is where it surfaced before loading moved earlier.
                let snapshot = extract_source_code(canonical, string_table);
                loaded.insert(canonical.clone(), snapshot);
            }
            queue.push_back(QueuedSource::Content {
                path: canonical.clone(),
                kind,
            });
            Ok(())
        }
        SingleFileReferenceOutcome::NoPhysicalTarget
        | SingleFileReferenceOutcome::Resource { .. }
        | SingleFileReferenceOutcome::Diagnostic(_) => Ok(()),
    }
}

/// Map one build-owned physical outcome onto the compiler-facing resolved row using the finished
/// classified identity table.
fn resolved_outcome_from_physical(
    resolved: SingleFileResolvedReference,
    source_files: &SourceDatabase,
    string_table: &mut StringTable,
) -> Result<ResolvedFileReferenceOutcome, CompilerMessages> {
    match resolved.outcome {
        SingleFileReferenceOutcome::NoPhysicalTarget => {
            Ok(ResolvedFileReferenceOutcome::NoPhysicalTarget)
        }
        SingleFileReferenceOutcome::Diagnostic(diagnostic) => {
            Ok(ResolvedFileReferenceOutcome::Diagnostic(diagnostic))
        }
        SingleFileReferenceOutcome::Resource {
            source,
            owner_relative_path,
        } => Ok(ResolvedFileReferenceOutcome::Target(
            ResolvedFileReferenceTarget::ResourceSource {
                source,
                owner_relative_path,
            },
        )),
        SingleFileReferenceOutcome::IdentifiedSourceKind => Ok(
            ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::IdentifiedSourceKind),
        ),
        SingleFileReferenceOutcome::Source { canonical } => {
            let source = source_id_for_path(
                source_files,
                &canonical,
                string_table,
                "has no classified source identity",
            )?;
            Ok(ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ContentSource { source },
            ))
        }
    }
}

fn source_id_for_path(
    source_files: &SourceDatabase,
    path: &Path,
    string_table: &mut StringTable,
    missing: &str,
) -> Result<SourceId, CompilerMessages> {
    source_files
        .get_by_canonical_path(path)
        .map(|identity| identity.id)
        .ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!("source {path:?} {missing}")),
                string_table,
            )
        })
}

fn prepare_one_source(
    source_files: &SourceDatabase,
    source_path: &Path,
    kind: SourceFileKind,
    source_code: &str,
    options: &HeaderParseOptions<'_>,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, CompilerMessages> {
    let context = FrontendFilePrepareContext {
        source_files,
        style_directives,
        entry_file_path: source_path,
        options,
    };
    let source = match kind {
        SourceFileKind::MothTemplate => FrontendFilePrepareSource::MothTemplate {
            source_code,
            source_path: source_path.to_path_buf(),
        },
        SourceFileKind::PlainMarkdown => FrontendFilePrepareSource::PlainMarkdown {
            source_code,
            source_path: source_path.to_path_buf(),
        },
        SourceFileKind::Moth => {
            return Err(CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "direct template content closure reached a Moth module source",
                ),
                string_table,
            ));
        }
    };
    let input = FrontendFilePrepareInput {
        source,
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    CompilerFrontend::prepare_file_frontend_local(&context, input, string_table).map_err(|error| {
        match error {
            FileFrontendPrepareFailure::Diagnosed(error) => {
                let mut messages =
                    CompilerMessages::from_diagnostic(*error.diagnostic, string_table.clone());
                messages.prepend_diagnostics_preserving_context(error.warnings);
                messages
            }
            FileFrontendPrepareFailure::Infrastructure(error) => {
                CompilerMessages::from_error(error, string_table.clone())
            }
        }
    })
}

fn recognised_source_file_kinds() -> SourceFileKindRegistry {
    let mut registry = SourceFileKindRegistry::new();
    for supported in SourceFileKind::recognized_kinds() {
        registry.register(supported.extension, supported.kind);
    }
    registry
}

fn incompatible_class_messages(string_table: &mut StringTable) -> CompilerMessages {
    CompilerMessages::from_error_ref(
        CompilerError::compiler_error(
            "direct template content closure retained an incompatible physical outcome class",
        ),
        string_table,
    )
}

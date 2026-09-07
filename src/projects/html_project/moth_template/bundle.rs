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

    // ------------------------
    //  Discover the source closure
    // ------------------------
    //
    // Discovery identities are private until the complete content closure is known. Every
    // diagnostic exit below retains the known snapshots and builders through the same finalizer
    // used by the successful handoff.
    let mut discovery_files = SourceDatabase::empty();
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

    let mut sources = DiscoveredTemplateSources::default();
    sources
        .candidates
        .insert(unit.source_path.clone(), SourceFileKind::MothTemplate);
    sources.loaded.insert(
        unit.source_path.clone(),
        Ok(std::mem::take(&mut unit.source_text)),
    );
    let mut queue = VecDeque::new();
    queue.push_back(QueuedSource::Entry);
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

        let source_code = match sources.loaded.get(&path) {
            Some(Ok(source_code)) => source_code.as_str(),
            Some(Err(error)) => {
                return Err(finalize_discovery_failure(
                    path,
                    FileFrontendPrepareFailure::Infrastructure(error.clone()),
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
            None => {
                return Err(finalize_discovery_failure(
                    path.clone(),
                    FileFrontendPrepareFailure::Infrastructure(CompilerError::compiler_error(
                        format!("content source {path:?} has no retained source text"),
                    )),
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };

        // Register immediately before preparation so every token stream receives a
        // traversal-local identity.
        if let Err(error) = discovery_files.insert(
            path.clone(),
            SourceKind::Compiler(kind),
            &unit.source_path,
            Some(&path_resolver),
            string_table,
        ) {
            return Err(finalize_discovery_failure(
                path,
                FileFrontendPrepareFailure::Infrastructure(error),
                sources,
                &unit.source_path,
                &path_resolver,
                string_table,
            ));
        }

        let prepared = match prepare_one_source(
            &discovery_files,
            &path,
            kind,
            source_code,
            &discovery_options,
            style_directives,
            string_table,
        ) {
            Ok(prepared) => prepared,
            Err(failure) => {
                return Err(finalize_discovery_failure(
                    path,
                    failure,
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };
        let resolved_references = {
            let path_syntax_table = prepared.path_syntax.table();
            let structural_references = prepared.structural_file_references.references();
            let mut resolved_references = Vec::with_capacity(structural_references.len());
            let mut failure = None;

            for reference in structural_references {
                let resolved =
                    match resolver.resolve(&path, path_syntax_table, reference, string_table) {
                        Ok(resolved) => resolved,
                        Err(error) => {
                            failure = Some(error);
                            break;
                        }
                    };
                if let Err(error) = collect_content_candidate(
                    &resolved,
                    &mut sources.candidates,
                    &mut sources.loaded,
                    &mut queue,
                    &source_file_kinds,
                    string_table,
                ) {
                    failure = Some(error);
                    break;
                }
                resolved_references.push(resolved);
            }

            match failure {
                Some(error) => Err(error),
                None => Ok(resolved_references),
            }
        };
        let resolved_references = match resolved_references {
            Ok(resolved_references) => resolved_references,
            Err(error) => {
                sources.prepared.push((path.clone(), prepared));
                return Err(finalize_discovery_failure(
                    path,
                    FileFrontendPrepareFailure::Infrastructure(error),
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };
        pending_references.extend(resolved_references);
        sources.prepared.push((path, prepared));
    }

    // The source database owns the final canonical identities. Prepared outputs stay live and
    // retain their original span builders for the compiler service to install after folding.
    let FinalizedTemplateSources {
        source_files,
        prepared_entry,
        prepared_content_sources,
    } = finalize_known_sources(sources, &unit.source_path, &path_resolver, string_table)?;
    let prepared_entry = prepared_entry.ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "direct-template walk completed without a prepared entry source",
            ),
            string_table,
        )
    })?;

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
        prepared_entry,
        prepared_content_sources,
        resolved_file_references,
        source_files,
        module_origin: Some(module_origin),
    })
}

/// The final source database and prepared outputs after canonical identity rebinding.
///
/// Success requires the entry output and hands it to the compiler with its original span builder
/// still live. Diagnosed discovery may have no entry output when the entry producer itself failed,
/// so it installs the failed producer's builder separately.
struct FinalizedTemplateSources {
    source_files: SourceDatabase,
    prepared_entry: Option<FileFrontendPrepareOutput>,
    prepared_content_sources: Vec<FileFrontendPrepareOutput>,
}

/// Discovery owns the known snapshots and their original preparations until one terminal handoff.
#[derive(Default)]
struct DiscoveredTemplateSources {
    candidates: FxHashMap<PathBuf, SourceFileKind>,
    loaded: FxHashMap<PathBuf, Result<String, CompilerError>>,
    prepared: Vec<(PathBuf, FileFrontendPrepareOutput)>,
}

/// Finalize all source candidates known at a discovery boundary.
///
/// The same path is used after a complete walk and after an aborted walk. It moves every retained
/// snapshot into the final identity table and rebinds every prepared output exactly once; it does
/// not install span builders because successful compiler folding remains their last producer.
fn finalize_known_sources(
    sources: DiscoveredTemplateSources,
    entry_file_path: &Path,
    path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<FinalizedTemplateSources, CompilerMessages> {
    let DiscoveredTemplateSources {
        candidates,
        loaded,
        prepared,
    } = sources;
    let registration_index = SourceRegistrationIndex::from_rows(
        candidates
            .iter()
            .map(|(path, kind)| (path.as_path(), SourceKind::Compiler(*kind))),
    );
    let mut source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
        &registration_index,
        entry_file_path,
        Some(path_resolver),
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

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
            Err(mut error) => {
                error
                    .location
                    .rebind_source_identity(&source_files.legacy_logical_path(source_id));
                source_files
                    .record_source_load_error(source_id, error)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
            }
        }
    }

    let mut prepared_entry = None;
    let mut prepared_content_sources = Vec::new();
    for (path, mut prepared) in prepared {
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

        if path == entry_file_path {
            prepared_entry = Some(prepared);
        } else {
            prepared_content_sources.push(prepared);
        }
    }
    Ok(FinalizedTemplateSources {
        source_files,
        prepared_entry,
        prepared_content_sources,
    })
}

/// Install one prepared source's original span builder after aborted discovery.
fn install_prepared_span_builder(
    source_files: &mut SourceDatabase,
    prepared: &mut FileFrontendPrepareOutput,
) -> Result<(), CompilerError> {
    let span_builder = std::mem::take(&mut prepared.span_builder);
    source_files.install_extended_spans(prepared.file_id, span_builder.freeze())
}

/// Finish an aborted discovery walk without dropping known source ownership.
///
/// Diagnosed preparation keeps the producer's typed warning, diagnostic and span builder until
/// this boundary. Once no later producer can append a row, every known source is finalized, the
/// original builders are installed, and the finalized database is attached to the message set.
fn finalize_discovery_failure(
    failed_path: PathBuf,
    failure: FileFrontendPrepareFailure,
    sources: DiscoveredTemplateSources,
    entry_file_path: &Path,
    path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let finalized =
        match finalize_known_sources(sources, entry_file_path, path_resolver, string_table) {
            Ok(finalized) => finalized,
            Err(messages) => return messages,
        };
    let FinalizedTemplateSources {
        mut source_files,
        mut prepared_entry,
        mut prepared_content_sources,
    } = finalized;

    // Final identity rebinding updates warning locations along with the prepared output. Move
    // those warnings only after rebinding so a diagnosed boundary never carries traversal-local
    // source scopes into its finalized database.
    let mut prior_warnings = Vec::new();
    if let Some(prepared_entry) = &mut prepared_entry {
        prior_warnings.append(&mut prepared_entry.warnings);
    }
    for prepared in &mut prepared_content_sources {
        prior_warnings.append(&mut prepared.warnings);
    }

    if let Some(prepared_entry) = &mut prepared_entry
        && let Err(error) = install_prepared_span_builder(&mut source_files, prepared_entry)
    {
        let mut messages =
            CompilerMessages::from_error_with_warnings(error, prior_warnings, string_table);
        messages.set_source_database(Arc::new(source_files));
        return messages;
    }
    for prepared in &mut prepared_content_sources {
        if let Err(error) = install_prepared_span_builder(&mut source_files, prepared) {
            let mut messages =
                CompilerMessages::from_error_with_warnings(error, prior_warnings, string_table);
            messages.set_source_database(Arc::new(source_files));
            return messages;
        }
    }

    let source_id = match source_id_for_path(
        &source_files,
        &failed_path,
        string_table,
        "has no finalized identity for its preparation failure",
    ) {
        Ok(source_id) => source_id,
        Err(messages) => return messages,
    };
    let logical_path = source_files.legacy_logical_path(source_id);

    let mut messages = match failure {
        FileFrontendPrepareFailure::Diagnosed(mut error) => {
            for warning in &mut error.warnings {
                warning.rebind_source_identity(&logical_path);
            }
            error.diagnostic.rebind_source_identity(&logical_path);
            prior_warnings.extend(error.warnings);

            if let Err(install_error) =
                source_files.install_extended_spans(source_id, error.span_builder.freeze())
            {
                let mut messages = CompilerMessages::from_error_with_warnings(
                    install_error,
                    prior_warnings,
                    string_table,
                );
                messages.set_source_database(Arc::new(source_files));
                return messages;
            }

            CompilerMessages::from_diagnostic_with_warnings(
                *error.diagnostic,
                prior_warnings,
                string_table,
            )
        }
        FileFrontendPrepareFailure::Infrastructure(mut error) => {
            error.location.rebind_source_identity(&logical_path);
            CompilerMessages::from_error_with_warnings(error, prior_warnings, string_table)
        }
    };
    messages.set_source_database(Arc::new(source_files));
    messages
}

/// Record one physically resolved content target in the candidate set and queue it.
///
/// Loading happens here, before the caller's visited check, so it must dedupe on the loaded
/// snapshot rather than on the queue. A path already collected keeps the kind its own producer
/// authored. Only a genuinely new path is classified here, because a lexical spelling can resolve
/// onto a target whose extension names another kind, and asserting a second kind for one path
/// would make the source registration ambiguous.
fn collect_content_candidate(
    resolved: &SingleFileResolvedReference,
    candidates: &mut FxHashMap<PathBuf, SourceFileKind>,
    loaded: &mut FxHashMap<PathBuf, Result<String, CompilerError>>,
    queue: &mut VecDeque<QueuedSource>,
    source_file_kinds: &SourceFileKindRegistry,
    string_table: &mut StringTable,
) -> Result<(), CompilerError> {
    match &resolved.outcome {
        SingleFileReferenceOutcome::IdentifiedSourceKind => {
            if resolved.class != PreparedFileReferenceClass::SourceKindNoFileValue {
                return Err(incompatible_class_error());
            }
            Ok(())
        }
        SingleFileReferenceOutcome::Source { canonical } => {
            if resolved.class != PreparedFileReferenceClass::ContentSource {
                return Err(incompatible_class_error());
            }

            let kind = if let Some(&kind) = candidates.get(canonical) {
                kind
            } else {
                let extension = canonical
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default();
                let Some(kind) = source_file_kinds.kind_for_extension(extension) else {
                    return Err(CompilerError::compiler_error(
                        "resolved supported content target has no registered source kind",
                    ));
                };
                candidates.insert(canonical.clone(), kind);
                kind
            };

            if !loaded.contains_key(canonical) {
                // A read failure is recorded against the snapshot rather than returned here. The
                // reference is still queued so the failure surfaces when this source is popped
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
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
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
            return Err(FileFrontendPrepareFailure::Infrastructure(
                CompilerError::compiler_error(
                    "direct template content closure reached a Moth module",
                ),
            ));
        }
    };
    let input = FrontendFilePrepareInput {
        source,
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    CompilerFrontend::prepare_file_frontend_local(&context, input, string_table)
}

fn recognised_source_file_kinds() -> SourceFileKindRegistry {
    let mut registry = SourceFileKindRegistry::new();
    for supported in SourceFileKind::recognized_kinds() {
        registry.register(supported.extension, supported.kind);
    }
    registry
}

fn incompatible_class_error() -> CompilerError {
    CompilerError::compiler_error(
        "direct template content closure retained an incompatible physical outcome class",
    )
}

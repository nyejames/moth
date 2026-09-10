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
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderParseOptions,
    SourcePreparationDelta,
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
    ExtendedSpanBuilder, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceKind,
    SourceRegistrationIndex, SourceSpan,
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
                    None,
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
                    None,
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };

        // Register immediately before preparation so every token stream receives a
        // traversal-local identity.
        let source_id = match discovery_files.insert(
            path.clone(),
            SourceKind::Compiler(kind),
            &unit.source_path,
            Some(&path_resolver),
            string_table,
        ) {
            Ok(source_id) => source_id,
            Err(error) => {
                return Err(finalize_discovery_failure(
                    path,
                    FileFrontendPrepareFailure::Infrastructure(error),
                    None,
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };

        let SourcePreparationDelta {
            file_id: _,
            span_builder,
            result,
        } = match prepare_one_source(
            &FrontendFilePrepareContext {
                source_files: &discovery_files,
                style_directives,
                entry_file_path: &path,
                options: &discovery_options,
            },
            source_id,
            kind,
            source_code,
            string_table,
        ) {
            Ok(delta) => delta,
            Err(failure) => {
                return Err(finalize_discovery_failure(
                    path,
                    failure,
                    None,
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(failure) => {
                return Err(finalize_discovery_failure(
                    path,
                    failure,
                    Some(span_builder),
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
                sources
                    .prepared
                    .push((path.clone(), prepared, Some(span_builder)));
                return Err(finalize_discovery_failure(
                    path,
                    FileFrontendPrepareFailure::Infrastructure(error),
                    None,
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                ));
            }
        };
        pending_references.extend(resolved_references);
        sources.prepared.push((path, prepared, Some(span_builder)));
    }

    // The exclusive source owner holds the final canonical identities and every retained
    // original span builder. Prepared outputs travel builder-free; the compiler service installs
    // all tables after folding under the owner's exclusive control.
    let FinalizedTemplateSources {
        source_builder,
        prepared_entry,
        prepared_content_sources,
    } = finalize_known_sources(
        sources,
        None,
        &unit.source_path,
        &path_resolver,
        string_table,
    )?;
    let Some(prepared_entry) = prepared_entry else {
        let messages = CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "direct-template walk completed without a prepared entry source",
            ),
            string_table,
        );
        return Err(finish_source_owner(messages, source_builder, string_table));
    };

    let mut resolved_file_references = ResolvedFileReferenceTable::new();
    for resolved in pending_references {
        // Discovery prepared this reference's owner with a traversal-local SourceId. The shared
        // path resolver computes the same logical path during final registration, but final
        // sorted identities can differ. Prepared outputs were rebound above. The diagnostic is
        // separately owned, so keep its identity normalization alongside the final row's SourceId
        // assignment even though the logical path is already final.
        let owner_source_file = match source_builder
            .sources()
            .get_by_canonical_path(&resolved.source_path)
            .map(|owner| owner.id)
        {
            Some(owner_source_file) => owner_source_file,
            None => {
                let messages = CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "source {:?} owner has no classified source identity",
                        resolved.source_path
                    )),
                    string_table,
                );
                return Err(finish_source_owner(messages, source_builder, string_table));
            }
        };
        let path_syntax = resolved.path_syntax;
        let class = resolved.class;
        let mut outcome = match resolved_outcome_from_physical(
            resolved,
            source_builder.sources(),
            string_table,
        ) {
            Ok(outcome) => outcome,
            Err(messages) => {
                return Err(finish_source_owner(messages, source_builder, string_table));
            }
        };
        if let ResolvedFileReferenceOutcome::Diagnostic(diagnostic) = &mut outcome {
            rebind_diagnostic_source(diagnostic, owner_source_file);
        }
        if let Err(error) = resolved_file_references.push(ResolvedFileReference {
            source_file: owner_source_file,
            path_syntax,
            class,
            outcome,
        }) {
            let messages = CompilerMessages::from_error_ref(error, string_table);
            return Err(finish_source_owner(messages, source_builder, string_table));
        }
    }

    Ok(MothTemplateFileValueBundle {
        prepared_entry,
        prepared_content_sources,
        resolved_file_references,
        source_files: source_builder,
        module_origin: Some(module_origin),
    })
}

/// The exclusive source owner and builder-free prepared outputs after identity rebinding.
///
/// Success requires the entry output. Diagnosed discovery may have no entry output when the
/// entry producer itself failed, so its builder arrives through the failure's
/// `SourcePreparationDelta` in the failure finalizer instead.
struct FinalizedTemplateSources {
    source_builder: SourceDatabaseBuilder,
    prepared_entry: Option<FileFrontendPrepareOutput>,
    prepared_content_sources: Vec<FileFrontendPrepareOutput>,
}

/// Discovery owns the known snapshots and their original preparations until one terminal handoff.
#[derive(Default)]
struct DiscoveredTemplateSources {
    candidates: FxHashMap<PathBuf, SourceFileKind>,
    loaded: FxHashMap<PathBuf, Result<String, CompilerError>>,
    prepared: Vec<(
        PathBuf,
        FileFrontendPrepareOutput,
        Option<ExtendedSpanBuilder>,
    )>,
}

/// The same path is used after a complete walk and after an aborted walk. It moves every retained
/// snapshot into the final identity table, adopts an optional failed producer builder, rebinds
/// every prepared output exactly once and retains each original span builder under its final
/// identity; it does not install span tables because successful compiler folding remains their
/// last producer.
fn finalize_known_sources(
    sources: DiscoveredTemplateSources,
    failed_producer: Option<(&Path, ExtendedSpanBuilder)>,
    entry_file_path: &Path,
    path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<FinalizedTemplateSources, CompilerMessages> {
    let DiscoveredTemplateSources {
        candidates,
        loaded,
        mut prepared,
    } = sources;
    let registration_index = SourceRegistrationIndex::from_rows(
        candidates
            .iter()
            .map(|(path, kind)| (path.as_path(), SourceKind::Compiler(*kind))),
    );
    let source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
        &registration_index,
        entry_file_path,
        Some(path_resolver),
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let mut source_builder = SourceDatabaseBuilder::new(source_files);
    let mut transfer_error = None;

    // Move every known snapshot into final ownership before touching any prepared output. A
    // failed transfer is remembered while the remaining siblings still cross the ownership
    // boundary, so a later terminal error cannot drop an otherwise valid live builder.
    for (path, snapshot) in loaded {
        let source_id = match source_builder
            .sources()
            .get_by_canonical_path(&path)
            .map(|record| record.id)
        {
            Some(source_id) => source_id,
            None => {
                if transfer_error.is_none() {
                    transfer_error = Some(CompilerError::compiler_error(format!(
                        "source {path:?} has no identity after classified registration",
                    )));
                }
                continue;
            }
        };
        match snapshot {
            Ok(source_code) => {
                if let Err(error) = source_builder
                    .sources_mut()
                    .retain_text(source_id, source_code)
                {
                    transfer_error.get_or_insert(error);
                }
            }
            Err(error) => {
                if let Err(install_error) = source_builder
                    .sources_mut()
                    .record_source_load_error(source_id, error)
                {
                    transfer_error.get_or_insert(install_error);
                }
            }
        }
    }

    // Retain every original producer builder before the first fallible output rebinding. The
    // loaded-record check avoids asking the owner to retain a builder for a snapshot transfer
    // that itself failed.
    for (path, _, span_builder) in &mut prepared {
        let source_id = match source_builder
            .sources()
            .get_by_canonical_path(path)
            .map(|record| record.id)
        {
            Some(source_id) => source_id,
            None => {
                if transfer_error.is_none() {
                    transfer_error = Some(CompilerError::compiler_error(format!(
                        "source {path:?} has no identity before retaining its span builder",
                    )));
                }
                continue;
            }
        };
        if source_builder.sources().line_index(source_id).is_none() {
            if transfer_error.is_none() {
                transfer_error = Some(CompilerError::compiler_error(format!(
                    "source {path:?} has no retained text before retaining its span builder",
                )));
            }
            continue;
        }
        let Some(span_builder) = span_builder.take() else {
            if transfer_error.is_none() {
                transfer_error = Some(CompilerError::compiler_error(format!(
                    "source {path:?} lost its original span builder before final ownership",
                )));
            }
            continue;
        };
        source_builder.retain_span_builder(source_id, span_builder);
    }
    if let Some((failed_path, span_builder)) = failed_producer {
        match source_builder.sources().get_by_canonical_path(failed_path) {
            Some(record) if source_builder.sources().line_index(record.id).is_some() => {
                source_builder.retain_span_builder(record.id, span_builder);
            }
            _ => {
                transfer_error.get_or_insert_with(|| CompilerError::compiler_error(format!(
                    "failed source producer {failed_path:?} has no loaded final source identity",
                )));
            }
        }
    }

    if let Some(error) = transfer_error {
        let messages = CompilerMessages::from_error_ref(error, string_table);
        return Err(finish_source_owner(messages, source_builder, string_table));
    }
    let rebound: Result<_, CompilerError> = (|| {
        let mut prepared_entry = None;
        let mut prepared_content_sources = Vec::new();
        for (path, mut prepared, _) in prepared {
            let source_id = source_builder
                .sources()
                .get_by_canonical_path(&path)
                .expect("transferred source must remain registered")
                .id;
            let is_entry = path == entry_file_path;
            prepared.rebind_source_identity(
                source_id,
                source_builder.sources().legacy_logical_path(source_id),
                path,
            )?;
            prepared.freeze_path_syntax(string_table)?;
            if is_entry {
                prepared_entry = Some(prepared);
            } else {
                prepared_content_sources.push(prepared);
            }
        }
        Ok((prepared_entry, prepared_content_sources))
    })();
    match rebound {
        Ok((prepared_entry, prepared_content_sources)) => Ok(FinalizedTemplateSources {
            source_builder,
            prepared_entry,
            prepared_content_sources,
        }),
        Err(error) => {
            let messages = CompilerMessages::from_error_ref(error, string_table);
            Err(finish_source_owner(messages, source_builder, string_table))
        }
    }
}

/// Finish the exclusive source owner and attach the finalized context to one terminal message
/// set. If span installation itself fails, preserve that infrastructure failure and the original
/// diagnostics as the ordered message payload.
fn finish_source_owner(
    mut messages: CompilerMessages,
    source_builder: SourceDatabaseBuilder,
    string_table: &StringTable,
) -> CompilerMessages {
    match source_builder.finish() {
        Ok(source_files) => {
            messages.set_source_database(Arc::new(source_files));
            messages
        }
        Err(install_error) => CompilerMessages::from_error_with_warnings(
            install_error,
            messages.into_diagnostics(),
            string_table,
        ),
    }
}

/// Finish an aborted discovery walk without dropping known source ownership.
///
/// Diagnosed preparation keeps the producer's typed warning and diagnostic until this boundary.
/// Once no later producer can append a row, every known source is finalized under the exclusive
/// owner, its original builders and the failed producer's delta builder are installed, and the
/// finalized database is attached to the message set.
fn finalize_discovery_failure(
    failed_path: PathBuf,
    mut failure: FileFrontendPrepareFailure,
    failed_builder: Option<ExtendedSpanBuilder>,
    sources: DiscoveredTemplateSources,
    entry_file_path: &Path,
    path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let finalized = match finalize_known_sources(
        sources,
        failed_builder.map(|builder| (failed_path.as_path(), builder)),
        entry_file_path,
        path_resolver,
        string_table,
    ) {
        Ok(finalized) => finalized,
        Err(messages) => return messages,
    };
    let FinalizedTemplateSources {
        source_builder,
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

    let source_id = match source_id_for_path(
        source_builder.sources(),
        &failed_path,
        string_table,
        "has no finalized identity for its preparation failure",
    ) {
        Ok(source_id) => source_id,
        Err(mut messages) => {
            messages.prepend_diagnostics_preserving_context(prior_warnings);
            return finish_source_owner(messages, source_builder, string_table);
        }
    };
    match &mut failure {
        FileFrontendPrepareFailure::Diagnosed(error) => {
            for warning in &mut error.warnings {
                rebind_diagnostic_source(warning, source_id);
            }
            rebind_diagnostic_source(&mut error.diagnostic, source_id);
            prior_warnings.append(&mut error.warnings);
        }
        FileFrontendPrepareFailure::Infrastructure(error) => {
            rebind_error_source(error, source_id);
        }
    }

    let messages = match failure {
        FileFrontendPrepareFailure::Diagnosed(error) => {
            CompilerMessages::from_diagnostic_with_warnings(
                error.diagnostic,
                prior_warnings,
                string_table,
            )
        }
        FileFrontendPrepareFailure::Infrastructure(error) => {
            CompilerMessages::from_error_with_warnings(error, prior_warnings, string_table)
        }
    };
    finish_source_owner(messages, source_builder, string_table)
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

fn rebind_source_span(span: &mut Option<SourceSpan>, source_id: SourceId) {
    if let Some(span) = span
        && span.source() != SourceId::COMPILATION_ROOT
    {
        *span = SourceSpan::new(source_id, span.local());
    }
}

fn rebind_diagnostic_source(diagnostic: &mut CompilerDiagnostic, source_id: SourceId) {
    rebind_source_span(&mut diagnostic.primary_span, source_id);
    for label in &mut diagnostic.labels {
        rebind_source_span(&mut label.span, source_id);
    }
}

fn rebind_error_source(error: &mut CompilerError, source_id: SourceId) {
    rebind_source_span(&mut error.source_span, source_id);
}

fn prepare_one_source(
    context: &FrontendFilePrepareContext<'_>,
    source_id: SourceId,
    kind: SourceFileKind,
    source_code: &str,
    string_table: &mut StringTable,
) -> Result<SourcePreparationDelta, FileFrontendPrepareFailure> {
    let source_path = context.entry_file_path;
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
        source_id,
        span_builder: ExtendedSpanBuilder::new(),
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    Ok(CompilerFrontend::prepare_file_frontend_local(
        context,
        input,
        string_table,
    ))
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

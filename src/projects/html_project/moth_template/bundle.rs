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
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, DiagnosticDraft, PreparedFailure,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure, FileFrontendPrepareOutput,
    HeaderParseOptions, SourcePreparationDelta,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceFailure,
    ResolvedFileReferenceOutcome, ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::single_source_compilation::MothTemplateFileValueBundle;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabase, SourceDatabaseBuilder, SourceDatabaseError, SourceId,
    SourceKind, SourceRegistrationIndex, SourceSpan,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
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
#[derive(Debug)]
pub(super) struct MothTemplateBundleFailure {
    failure: PreparedFailure,
    source_database: Option<Arc<SourceDatabase>>,
}

impl From<PreparedFailure> for MothTemplateBundleFailure {
    fn from(failure: PreparedFailure) -> Self {
        Self {
            failure,
            source_database: None,
        }
    }
}

impl MothTemplateBundleFailure {
    pub(super) fn append_to_boundary(self, accumulator: &mut BoundaryAccumulator) {
        match self.failure {
            PreparedFailure::Diagnosed(batch) => accumulator
                .append_prepared(batch, self.source_database.map(|database| (database, None))),
            PreparedFailure::Infrastructure(errors) => {
                accumulator.append_infrastructure(errors, self.source_database);
            }
            PreparedFailure::Mixed { batch, error } => {
                let source_database = self.source_database;
                accumulator.append_prepared(
                    batch,
                    source_database
                        .as_ref()
                        .map(|database| (Arc::clone(database), None)),
                );
                accumulator.append_infrastructure(error, source_database);
            }
        }
    }
}

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
) -> Result<MothTemplateFileValueBundle, MothTemplateBundleFailure> {
    let module_root = unit
        .source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let source_file_kinds = recognised_source_file_kinds();
    let path_resolver = ProjectPathResolver::new_with_module_roots(
        module_root.clone(),
        module_root.clone(),
        PreparedSourcePackageRoots::default(),
        &source_file_kinds,
        ModuleRootTable::empty(),
    )
    .map_err(PreparedFailure::from)?;

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
    .map_err(PreparedFailure::from)?;

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
    let mut path_fork = PathInternerFork::empty();

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
        .map_err(|error| match error {
            SourceDatabaseError::Capacity(capacity) => PreparedFailure::capture_diagnosed(
                DiagnosticBag::from(token_lexical::source_span_capacity(
                    0,
                    u32::MAX,
                    capacity.resource(),
                    None,
                )),
                string_table.clone(),
                None,
                false,
                None,
            ),
            SourceDatabaseError::Infrastructure(error) => PreparedFailure::from(error),
        })?;
    let discovery_options = HeaderParseOptions {
        entry_file_id: Some(entry_file_id),
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
                    &mut path_fork,
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
                    &mut path_fork,
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
                let failure = match error {
                    // Discovery registers the authored template closure, so its compact-table
                    // exhaustion is a typed source-capacity rejection, not infrastructure.
                    SourceDatabaseError::Capacity(capacity) => {
                        FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                            warnings: DiagnosticBag::new(),
                            diagnostic: CompilerDiagnostic::source_span_capacity(
                                0,
                                u32::MAX,
                                capacity.resource(),
                                None,
                            ),
                        })
                    }
                    SourceDatabaseError::Infrastructure(error) => {
                        FileFrontendPrepareFailure::Infrastructure(error)
                    }
                };
                return Err(finalize_discovery_failure(
                    path,
                    failure,
                    None,
                    sources,
                    &unit.source_path,
                    &path_resolver,
                    string_table,
                    &mut path_fork,
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
            &mut path_fork,
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
                    &mut path_fork,
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
                    &mut path_fork,
                ));
            }
        };
        let resolved_references = {
            let path_syntax_table = prepared.path_syntax.table();
            let structural_references = prepared.structural_file_references.references();
            let mut resolved_references = Vec::with_capacity(structural_references.len());
            let mut failure = None;

            for reference in structural_references {
                let resolved = match resolver.resolve(
                    &path,
                    path_syntax_table,
                    reference,
                    string_table,
                    &path_fork,
                ) {
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
                    &mut path_fork,
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
        &mut path_fork,
    )?;
    let Some(prepared_entry) = prepared_entry else {
        let failure = PreparedFailure::from(CompilerError::compiler_error(
            "direct-template walk completed without a prepared entry source",
        ));
        return Err(finish_source_owner(failure, source_builder));
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
                let failure = PreparedFailure::from(CompilerError::compiler_error(format!(
                    "source {:?} owner has no classified source identity",
                    resolved.source_path
                )));
                return Err(finish_source_owner(failure, source_builder));
            }
        };
        let path_syntax = resolved.path_syntax;
        let class = resolved.class;
        let mut outcome = match resolved_outcome_from_physical(resolved, source_builder.sources()) {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(finish_source_owner(
                    PreparedFailure::from(error),
                    source_builder,
                ));
            }
        };
        if let ResolvedFileReferenceOutcome::Diagnostic(failure) = &mut outcome {
            rebind_resolved_failure_source(failure, owner_source_file);
        }
        if let Err(error) = resolved_file_references.push(ResolvedFileReference {
            source_file: owner_source_file,
            path_syntax,
            class,
            outcome,
        }) {
            return Err(finish_source_owner(
                PreparedFailure::from(error),
                source_builder,
            ));
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
    path_fork: &mut PathInternerFork,
) -> Result<FinalizedTemplateSources, MothTemplateBundleFailure> {
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
    let source_files =
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &registration_index,
            entry_file_path,
            Some(path_resolver),
            string_table,
            path_fork.clone_path_builder(),
        )
        .map_err(|error| match error {
            // Final registration publishes the authored template closure, so its compact-table
            // exhaustion is a typed source-capacity rejection, not infrastructure.
            SourceDatabaseError::Capacity(capacity) => PreparedFailure::capture_diagnosed(
                DiagnosticBag::from(token_lexical::source_span_capacity(
                    0,
                    u32::MAX,
                    capacity.resource(),
                    None,
                )),
                string_table.clone(),
                None,
                false,
                None,
            ),
            SourceDatabaseError::Infrastructure(error) => PreparedFailure::from(error),
        })?;
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
        return Err(finish_source_owner(
            PreparedFailure::from(error),
            source_builder,
        ));
    }

    // Registration may append logical paths for candidates queued before an aborted discovery
    // walk reaches them. Reseed those source paths into the original fork in final source-slot
    // order, and reject any unexpected identity remap before rebinding retained outputs.
    let reseed_error: Option<PreparedFailure> = {
        let source_files = source_builder.sources();
        source_files.iter().find_map(|source| {
            let expected_path = source.logical_path;
            match source_files.logical_path_in_fork(source.id, path_fork) {
                Ok(path) if path == expected_path => None,
                Ok(_) => Some(PreparedFailure::from(CompilerError::compiler_error(
                    "final source registration changed an existing logical PathId",
                ))),
                Err(SourceDatabaseError::Capacity(capacity)) => {
                    Some(PreparedFailure::capture_diagnosed(
                        DiagnosticBag::from(token_lexical::source_span_capacity(
                            0,
                            u32::MAX,
                            capacity.resource(),
                            None,
                        )),
                        string_table.clone(),
                        None,
                        false,
                        None,
                    ))
                }
                Err(SourceDatabaseError::Infrastructure(error)) => {
                    Some(PreparedFailure::from(error))
                }
            }
        })
    };

    if let Some(failure) = reseed_error {
        return Err(finish_source_owner(failure, source_builder));
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
                source_builder
                    .sources()
                    .source_logical_path(source_id)
                    .expect("transferred source must retain logical path"),
                path_fork,
            )?;
            prepared.freeze_path_syntax(string_table, path_fork)?;
            if is_entry {
                prepared_entry = Some(prepared);
            } else {
                prepared_content_sources.push(prepared);
            }
        }
        Ok((prepared_entry, prepared_content_sources))
    })();
    source_builder
        .sources_mut()
        .adopt_path_builder(path_fork.clone_path_builder());
    match rebound {
        Ok((prepared_entry, prepared_content_sources)) => Ok(FinalizedTemplateSources {
            source_builder,
            prepared_entry,
            prepared_content_sources,
        }),
        Err(error) => Err(finish_source_owner(
            PreparedFailure::from(error),
            source_builder,
        )),
    }
}

fn append_infrastructure_cause(failure: PreparedFailure, error: CompilerError) -> PreparedFailure {
    match failure {
        PreparedFailure::Diagnosed(batch) => PreparedFailure::Mixed {
            batch,
            error: error.into(),
        },
        PreparedFailure::Infrastructure(mut errors) => {
            errors.push(error);
            PreparedFailure::Infrastructure(errors)
        }
        PreparedFailure::Mixed {
            batch,
            error: mut errors,
        } => {
            errors.push(error);
            PreparedFailure::Mixed {
                batch,
                error: errors,
            }
        }
    }
}

fn prepared_infrastructure_failure_with_warnings(
    error: CompilerError,
    warnings: DiagnosticBag,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> PreparedFailure {
    let warning_diagnostics = warnings.into_diagnostics();
    if warning_diagnostics.is_empty() {
        return PreparedFailure::from(error);
    }

    match PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from_diagnostics(warning_diagnostics),
        string_table.clone(),
        None,
        || Some(Arc::new(path_fork.snapshot_table())),
    ) {
        Ok(batch) => PreparedFailure::Mixed {
            batch,
            error: error.into(),
        },
        Err(capture_error) => {
            let mut errors = InfrastructureErrors::new(error);
            errors.push(capture_error);
            PreparedFailure::Infrastructure(Box::new(errors))
        }
    }
}

fn finish_source_owner(
    failure: PreparedFailure,
    source_builder: SourceDatabaseBuilder,
) -> MothTemplateBundleFailure {
    match source_builder.finish() {
        Ok(source_database) => MothTemplateBundleFailure {
            failure,
            source_database: Some(Arc::new(source_database)),
        },
        Err(error) => MothTemplateBundleFailure {
            failure: append_infrastructure_cause(failure, error),
            source_database: None,
        },
    }
}

/// Finish an aborted discovery walk without dropping known source ownership.
///
/// Diagnosed preparation keeps the producer's typed warning and diagnostic until this boundary.
/// Once no later producer can append a row, every known source is finalized under the exclusive
/// owner, its original builders and the failed producer's delta builder are installed, and the
/// finalized database is retained until the prepared failure is paired at the boundary.
#[allow(
    clippy::too_many_arguments,
    reason = "failure finalization keeps the failed path, failure payload, failed builder, discovered sources, entry path, resolver, and mutable string/path state as separate inputs"
)]
fn finalize_discovery_failure(
    failed_path: PathBuf,
    failure: FileFrontendPrepareFailure,
    failed_builder: Option<ExtendedSpanBuilder>,
    sources: DiscoveredTemplateSources,
    entry_file_path: &Path,
    path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MothTemplateBundleFailure {
    let finalized = match finalize_known_sources(
        sources,
        failed_builder.map(|builder| (failed_path.as_path(), builder)),
        entry_file_path,
        path_resolver,
        string_table,
        path_fork,
    ) {
        Ok(finalized) => finalized,
        Err(failure) => return failure,
    };
    let FinalizedTemplateSources {
        source_builder,
        mut prepared_entry,
        mut prepared_content_sources,
    } = finalized;

    let mut prior_warnings = DiagnosticBag::new();
    if let Some(prepared_entry) = &mut prepared_entry {
        for warning in prepared_entry.warnings.take_diagnostics() {
            prior_warnings.push(warning);
        }
    }
    for prepared in &mut prepared_content_sources {
        for warning in prepared.warnings.take_diagnostics() {
            prior_warnings.push(warning);
        }
    }

    let source_id = match source_id_for_path(
        source_builder.sources(),
        &failed_path,
        "has no finalized identity for its preparation failure",
    ) {
        Ok(source_id) => source_id,
        Err(error) => {
            let failure = prepared_infrastructure_failure_with_warnings(
                error,
                prior_warnings,
                string_table,
                path_fork,
            );
            return finish_source_owner(failure, source_builder);
        }
    };
    let failure = match failure {
        FileFrontendPrepareFailure::Diagnosed(mut error) => {
            let mut error_warnings = error.warnings.take_diagnostics();
            for warning in &mut error_warnings {
                rebind_draft_diagnostic_source(warning, source_id);
            }
            let mut diagnostic: DiagnosticDraft = error.diagnostic.into();
            rebind_draft_diagnostic_source(&mut diagnostic, source_id);
            for warning in error_warnings {
                prior_warnings.push(warning);
            }
            PreparedFailure::with_diagnostic_and_warnings(
                diagnostic,
                prior_warnings.into_diagnostics(),
                string_table,
                None,
                || Some(Arc::new(path_fork.snapshot_table())),
            )
        }
        FileFrontendPrepareFailure::Infrastructure(mut error) => {
            rebind_error_source(&mut error, source_id);
            prepared_infrastructure_failure_with_warnings(
                error,
                prior_warnings,
                string_table,
                path_fork,
            )
        }
    };
    finish_source_owner(failure, source_builder)
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
) -> Result<ResolvedFileReferenceOutcome, CompilerError> {
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
    missing: &str,
) -> Result<SourceId, CompilerError> {
    source_files
        .get_by_canonical_path(path)
        .map(|identity| identity.id)
        .ok_or_else(|| CompilerError::compiler_error(format!("source {path:?} {missing}")))
}

fn rebind_source_span(span: &mut Option<SourceSpan>, source_id: SourceId) {
    if let Some(span) = span
        && span.source() != SourceId::COMPILATION_ROOT
    {
        *span = SourceSpan::new(source_id, span.local());
    }
}

fn rebind_resolved_failure_source(failure: &mut ResolvedFileReferenceFailure, source_id: SourceId) {
    let span = failure.span_mut();
    if span.source() != SourceId::COMPILATION_ROOT {
        *span = SourceSpan::new(source_id, span.local());
    }
}

fn rebind_draft_diagnostic_source(diagnostic: &mut DiagnosticDraft, source_id: SourceId) {
    let mut span = diagnostic.primary_span();
    rebind_source_span(&mut span, source_id);
    diagnostic.set_primary_span(span);

    for index in 0..diagnostic.label_count() {
        let mut span = diagnostic.label_span(index);
        rebind_source_span(&mut span, source_id);
        diagnostic.set_label_span(index, span);
    }
}

fn rebind_error_source(error: &mut CompilerError, source_id: SourceId) {
    rebind_source_span(&mut error.source_span, source_id);
}

fn prepare_one_source(
    context: &FrontendFilePrepareContext<'_>,
    source_id: SourceId,
    kind: SourceFileKind,
    path_fork: &mut PathInternerFork,
    source_code: &str,
    string_table: &mut StringTable,
) -> Result<SourcePreparationDelta, FileFrontendPrepareFailure> {
    let source = match kind {
        SourceFileKind::MothTemplate => FrontendFilePrepareSource::MothTemplate { source_code },
        SourceFileKind::PlainMarkdown => FrontendFilePrepareSource::PlainMarkdown { source_code },
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
        path_fork,
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

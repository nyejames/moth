//! Stage 0 bundle construction for one direct Moth template compilation.
//!
//! WHAT: walks one template's content-source fixed point with the build-owned physical resolver,
//!       prepares every nested `.mtf`/`.md` dependency in deterministic discovery order and
//!       assembles the compiler service's file-value bundle.
//! WHY:  physical file-reference resolution stays build-owned. The compiler service folds settled
//!       Stage 0 facts and never probes the filesystem, while watch and invalidation policy stays
//!       with this direct API's callers.
//!
//! The fixed point mirrors synthetic single-file discovery: content targets resolve relative to
//! the compiling template's directory (this lane's entry root), nested `.mtf`/`.md` sources join
//! the same source identity table before their own preparation, and every prepared occurrence
//! keeps its settled outcome. Route and output placement are never built here.

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
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::projects::html_project::moth_template::input::MothTemplateSourceUnit;
use rustc_hash::FxHashSet;
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

    // 1. Register the template's identity so preparation stamps the final SourceId.
    let mut source_files = SourceDatabase::empty();
    source_files
        .insert(
            unit.source_path.clone(),
            &unit.source_path,
            Some(&path_resolver),
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let Some(entry_file_id) = source_files
        .get_by_canonical_path(&unit.source_path)
        .map(|identity| identity.id)
    else {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "direct Moth template did not register its own source identity",
            ),
            string_table,
        ));
    };

    // 2. Walk the content fixed point: nested `.mtf`/`.md` targets join the source set and are
    //    prepared in BFS order, so no second scan or parse exists for this lane.
    let options = HeaderParseOptions {
        entry_file_id: Some(entry_file_id),
        project_path_resolver: Some(path_resolver.clone()),
        entry_file_role: None,
        active_root_role: ModuleRootRole::Normal,
    };
    // The request owner's registry receives every source this resolver issues. Watch and
    // missing-target interests remain this lane's physical facts for the caller to use.
    let mut resolver =
        SingleFileReferenceResolver::new(module_root.clone(), &source_file_kinds, resource_inputs);

    let mut queue = VecDeque::new();
    queue.push_back(QueuedSource::Entry);
    let mut prepared_content_sources = Vec::new();
    let mut resolved_file_references = ResolvedFileReferenceTable::new();
    let mut visited = FxHashSet::default();

    while let Some(pending) = queue.pop_front() {
        let visit_path = match &pending {
            QueuedSource::Entry => unit.source_path.clone(),
            QueuedSource::Content { path, .. } => path.clone(),
        };
        if !visited.insert(visit_path) {
            continue;
        }
        let (path, kind, source_code, owner_source_file) = match pending {
            QueuedSource::Entry => {
                let path = unit.source_path.clone();
                let source_id = source_files
                    .get_by_canonical_path(&path)
                    .map(|identity| identity.id)
                    .ok_or_else(|| {
                        CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(format!(
                                "entry source {path:?} has no identity before source-text retention"
                            )),
                            string_table,
                        )
                    })?;
                source_files
                    .retain_text(source_id, std::mem::take(&mut unit.source_text))
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                let source_code = source_files.retained_text(source_id).ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "entry source {path:?} lost its retained source text"
                        )),
                        string_table,
                    )
                })?;
                (path, SourceFileKind::MothTemplate, source_code, source_id)
            }
            QueuedSource::Content { path, kind } => {
                let source_id = source_files
                    .get_by_canonical_path(&path)
                    .map(|identity| identity.id)
                    .ok_or_else(|| {
                        CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(format!(
                                "content source {path:?} has no identity before preparation"
                            )),
                            string_table,
                        )
                    })?;
                let source_code = match source_files.retained_text(source_id) {
                    Some(source_code) => source_code,
                    // Loading moved earlier than preparation, so a recorded read failure is
                    // reported here instead: this is the point at which the read failed before.
                    None => {
                        return Err(match source_files.source_load_error(source_id) {
                            Some(error) => {
                                CompilerMessages::from_error_ref(error.clone(), string_table)
                            }
                            None => CompilerMessages::from_error_ref(
                                CompilerError::compiler_error(format!(
                                    "content source {path:?} has no retained source text"
                                )),
                                string_table,
                            ),
                        });
                    }
                };
                (path, kind, source_code, source_id)
            }
        };

        // 3. Prepare one file against the settled identity table.
        let prepared = prepare_one_source(
            &source_files,
            &path,
            kind,
            source_code,
            &options,
            style_directives,
            string_table,
        )?;
        let path_syntax_table = prepared.path_syntax.table();
        let structural_references = prepared.structural_file_references.references();

        for reference in structural_references {
            let resolved = resolver
                .resolve(&path, path_syntax_table, reference, string_table)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            let outcome = settle_reference_outcome(
                resolved,
                &mut source_files,
                &path_resolver,
                &mut queue,
                &source_file_kinds,
                string_table,
            )?;
            resolved_file_references
                .push(ResolvedFileReference {
                    source_file: owner_source_file,
                    path_syntax: reference.path_syntax,
                    class: reference.class,
                    outcome,
                })
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        }

        // The template's own prepared output stays with the compiler service; only dependencies
        // are handed to it.
        if path != unit.source_path {
            prepared_content_sources.push(prepared);
        }
    }

    Ok(MothTemplateFileValueBundle {
        prepared_content_sources,
        resolved_file_references,
        source_files: Arc::new(source_files),
        module_origin: Some(module_origin),
    })
}

/// Map one build-owned physical outcome onto the compiler-facing resolved row, queueing content
/// targets so the fixed point reaches every nested dependency.
///
/// This is the direct lane's identity join: canonical target paths resolved before the full
/// source inventory was known become the bundle database's `SourceId`s here.
fn settle_reference_outcome(
    resolved: SingleFileResolvedReference,
    source_files: &mut SourceDatabase,
    path_resolver: &ProjectPathResolver,
    queue: &mut VecDeque<QueuedSource>,
    source_file_kinds: &SourceFileKindRegistry,
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
        SingleFileReferenceOutcome::IdentifiedSourceKind => {
            if resolved.class != PreparedFileReferenceClass::SourceKindNoFileValue {
                return Err(incompatible_class_messages(string_table));
            }

            Ok(ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::IdentifiedSourceKind,
            ))
        }
        SingleFileReferenceOutcome::Source { canonical } => {
            if resolved.class != PreparedFileReferenceClass::ContentSource {
                return Err(incompatible_class_messages(string_table));
            }

            // A nested content source is never re-prepared: the identity table dedupes, and the
            // caller's visited set skips a file already reached. Loading happens here, before that
            // visited check, so it must dedupe on the slot rather than on the queue.
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
            let target_file = source_files
                .insert(
                    canonical.clone(),
                    canonical.as_path(),
                    Some(path_resolver),
                    string_table,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            if source_files.retained_text(target_file).is_none()
                && source_files.source_load_error(target_file).is_none()
            {
                // A read failure is recorded against the slot rather than returned here. The
                // reference is still queued so the failure surfaces when this source is popped
                // for preparation, which is where it surfaced before loading moved earlier.
                let outcome = match extract_source_code(&canonical, string_table) {
                    Ok(source_code) => source_files.retain_text(target_file, source_code),
                    Err(error) => source_files.record_source_load_error(target_file, error),
                };
                outcome.map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            }
            queue.push_back(QueuedSource::Content {
                path: canonical,
                kind,
            });

            Ok(ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ContentSource {
                    source: target_file,
                },
            ))
        }
    }
}

fn prepare_one_source(
    source_files: &SourceDatabase,
    source_path: &Path,
    kind: SourceFileKind,
    source_code: &str,
    options: &HeaderParseOptions,
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

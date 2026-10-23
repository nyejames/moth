//! Direct Moth template compilation service.
//!
//! WHAT: the compiler-owned stage sequence for one `.mtf` source — tokenization, synthetic
//!       `content` header preparation, interface binding, local declaration ordering and AST
//!       folding — returning folded `content`, resource-identity facts, ordered preparation/AST
//!       warnings and the finalized source database.
//!
//! WHY:  tooling needs template content without artifact planning, HIR, borrow validation or
//!       output writing. That shorter path is a named compiler service rather than a
//!       project-owned stage sequence. Project code supplies source or a compiler-prepared bundle;
//!       it never implements preparation semantics, binds, orders or folds the template itself.
//!
//! This is not a second Moth template parser or compiler mode. It uses the same owners as an
//! integrated `.mtf` dependency and must never grow a parallel Markdown or template renderer.
//! Physical file-reference resolution stays with the calling project. Its Stage 0 bundle retains
//! the entry and content-source preparations with their original span builders, which this
//! service consumes without preparing again or probing the filesystem. Source collection, scope
//! policy and output packaging stay with the caller.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::ast::const_values::store::ConstValueVisit;
use crate::compiler_frontend::ast::{
    Ast, AstBuildContext, AstBuildInput, FileValueResolutionServices, Stage0ResolutionFacts,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, PreparedFailure,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure, FileFrontendPrepareOutput,
    HeaderParseOptions, HeaderPreparationFailure, SourcePreparationDelta, bind_module_headers,
    prepare_header_syntax,
};
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, SortedHeaders, resolve_module_dependencies,
};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source::SourceDatabaseError;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityHandle, SourceDatabase, SourceDatabaseBuilder, SourceId,
    SourceKind, SourceRegistrationIndex,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendBuildProfile, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// One Moth template source and the style vocabulary it folds against.
pub(crate) struct MothTemplateCompilationRequest<'a> {
    /// The canonical path of the template source, used for its file identity.
    pub(crate) source_path: &'a Path,
    /// Optional direct-request text. Bundle-bearing requests use retained database text.
    pub(crate) source_code: Option<String>,
    /// The calling project's style directives, already merged.
    ///
    /// WHY: which directives a template may use is project vocabulary, so the caller owns the
    ///      registry and the service never assembles one from a project.
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    /// Prepared Stage 0 file-value inputs, present when the source names file values.
    pub(crate) file_value_resolution: Option<MothTemplateFileValueBundle>,
}

/// Prepared file-value inputs one direct Moth template folds against.
///
/// WHAT: the calling project's Stage 0 bundle for one template — its prepared entry and content
///       dependencies, the settled physical outcome of every prepared file-reference occurrence,
///       and the source identities of the template with all its dependencies.
/// WHY: physical file-reference resolution stays build-owned. The service consumes settled facts
///      without filesystem probes, route placement or output placement.
pub(crate) struct MothTemplateFileValueBundle {
    /// Prepared entry source with its final identity and frozen path syntax.
    pub(crate) prepared_entry: FileFrontendPrepareOutput,
    /// Prepared content dependencies with frozen path syntax, in discovery order.
    pub(crate) prepared_content_sources: Vec<FileFrontendPrepareOutput>,
    /// One settled outcome per prepared file-reference occurrence across the template and its
    /// content dependencies, keyed by `source_files` identities.
    pub(crate) resolved_file_references: ResolvedFileReferenceTable,
    /// Exclusive owner of the template's and every content dependency's final identities. It
    /// retains each source's original builder — discovery-prepared or standalone — until the
    /// service installs every table after its last producer (folding or a diagnosed boundary).
    pub(crate) source_files: SourceDatabaseBuilder,
    /// The owning module origin resource pieces intern against.
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
}

/// The folded template a caller packages into its own output shape.
///
/// The content keeps resource and site-root anchors opaque until a builder supplies its link plan.
/// Plain text remains a dedicated `Text` value, so it does not allocate a piece vector.
pub(crate) struct FoldedMothTemplate {
    pub(crate) content: OwnedFoldedString,
    /// The folded module's resolved resource origins in interning order.
    pub(crate) module_resources: ModuleResourceTable,
    pub(crate) warnings: Option<PreparedDiagnosticBatch>,
    /// The finalized source snapshots and extended spans used by this template's diagnostics.
    pub(crate) source_database: Arc<SourceDatabase>,
}

/// A template failure that retains prepared diagnostics and its source owner.
#[derive(Debug)]
pub(crate) struct MothTemplateCompilationFailure {
    failure: PreparedFailure,
    source_database: Option<Arc<SourceDatabase>>,
}

impl From<PreparedFailure> for MothTemplateCompilationFailure {
    fn from(failure: PreparedFailure) -> Self {
        Self {
            failure,
            source_database: None,
        }
    }
}

impl MothTemplateCompilationFailure {
    pub(crate) fn append_to_boundary(self, accumulator: &mut BoundaryAccumulator) -> InstallMode {
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
        InstallMode::Unfrozen
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
) -> PreparedFailure {
    let warning_diagnostics = warnings.into_diagnostics();
    if warning_diagnostics.is_empty() {
        return PreparedFailure::from(error);
    }

    match PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from_diagnostics(warning_diagnostics),
        string_table.clone(),
        None,
        || None,
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
) -> MothTemplateCompilationFailure {
    match source_builder.finish() {
        Ok(source_database) => MothTemplateCompilationFailure {
            failure,
            source_database: Some(Arc::new(source_database)),
        },
        Err(error) => MothTemplateCompilationFailure {
            failure: append_infrastructure_cause(failure, error),
            source_database: None,
        },
    }
}

/// Compile one Moth template source to its folded `content` value.
///
/// The service stops at folded AST data, so the projection side results of AST construction are
/// unused and no HIR, borrow, target validation or output stage runs.
pub(crate) fn compile_moth_template_source(
    request: MothTemplateCompilationRequest<'_>,
    string_table: &mut StringTable,
) -> Result<FoldedMothTemplate, MothTemplateCompilationFailure> {
    let source_root = request
        .source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut source_file_kinds = SourceFileKindRegistry::new();
    source_file_kinds.register(
        SourceFileKind::MothTemplate.extension(),
        SourceFileKind::MothTemplate,
    );
    let path_resolver = ProjectPathResolver::new_with_module_roots(
        source_root.clone(),
        source_root,
        PreparedSourcePackageRoots::default(),
        &source_file_kinds,
        ModuleRootTable::empty(),
    )
    .map_err(PreparedFailure::from)?;

    let mut request = request;
    let mut direct_source_code = request.source_code.take();
    let file_value_resolution = request.file_value_resolution.take();
    let bundle_input = file_value_resolution.is_some();

    let (mut all_prepared, resolved_references, mut source_builder, module_origin) =
        match file_value_resolution {
            Some(MothTemplateFileValueBundle {
                prepared_entry,
                prepared_content_sources,
                resolved_file_references,
                source_files,
                module_origin,
            }) => {
                let mut prepared_sources = Vec::with_capacity(1 + prepared_content_sources.len());
                prepared_sources.push(prepared_entry);
                prepared_sources.extend(prepared_content_sources);
                (
                    prepared_sources,
                    Some(resolved_file_references),
                    source_files,
                    module_origin,
                )
            }
            None => {
                let registration_index = SourceRegistrationIndex::from_rows(std::iter::once((
                    request.source_path,
                    SourceKind::Compiler(SourceFileKind::MothTemplate),
                )));
                let source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
                    &registration_index,
                    request.source_path,
                    Some(&path_resolver),
                    string_table,
                )
                .map_err(|error| match error {
                    SourceDatabaseError::Capacity(capacity) => PreparedFailure::capture_diagnosed(
                        DiagnosticBag::from(CompilerDiagnostic::source_table_capacity(
                            capacity.resource(),
                        )),
                        string_table.clone(),
                        None,
                        false,
                        None,
                    ),
                    SourceDatabaseError::Infrastructure(error) => PreparedFailure::from(error),
                })?;
                let source_builder = SourceDatabaseBuilder::new(source_files);
                (Vec::new(), None, source_builder, None)
            }
        };
    let mut path_fork = source_builder.sources().fork_path_interner();
    let mut preparation_warnings = DiagnosticBag::new();
    for prepared_source in &mut all_prepared {
        for warning in prepared_source.warnings.take_diagnostics() {
            preparation_warnings.push(warning);
        }
    }

    if bundle_input && direct_source_code.is_some() {
        let failure = prepared_infrastructure_failure_with_warnings(
            CompilerError::compiler_error(
                "Moth template file-value bundle must own the retained source text",
            ),
            preparation_warnings,
            string_table,
        );
        return Err(finish_source_owner(failure, source_builder));
    }

    let entry_file_id = match source_builder
        .sources()
        .get_by_canonical_path(request.source_path)
    {
        Some(source_identity) => source_identity.id,
        None => {
            let failure = prepared_infrastructure_failure_with_warnings(
                CompilerError::compiler_error(
                    "Moth template source file table does not contain its own source",
                ),
                preparation_warnings,
                string_table,
            );
            return Err(finish_source_owner(failure, source_builder));
        }
    };
    if all_prepared
        .first()
        .is_some_and(|prepared_entry| prepared_entry.file_id != entry_file_id)
    {
        let failure = prepared_infrastructure_failure_with_warnings(
            CompilerError::compiler_error(
                "Moth template bundle entry does not match its registered source identity",
            ),
            preparation_warnings,
            string_table,
        );
        return Err(finish_source_owner(failure, source_builder));
    }
    if !bundle_input {
        let Some(source_code) = direct_source_code.take() else {
            let failure = prepared_infrastructure_failure_with_warnings(
                CompilerError::compiler_error("Moth template source has no retained source text"),
                preparation_warnings,
                string_table,
            );
            return Err(finish_source_owner(failure, source_builder));
        };
        if let Err(error) = source_builder
            .sources_mut()
            .retain_text(entry_file_id, source_code)
        {
            let failure = prepared_infrastructure_failure_with_warnings(
                error,
                preparation_warnings,
                string_table,
            );
            return Err(finish_source_owner(failure, source_builder));
        }
    } else if source_builder
        .sources()
        .retained_text(entry_file_id)
        .is_none()
    {
        let failure = prepared_infrastructure_failure_with_warnings(
            CompilerError::compiler_error("Moth template source has no retained source text"),
            preparation_warnings,
            string_table,
        );
        return Err(finish_source_owner(failure, source_builder));
    }
    let entry_scope = source_builder
        .sources()
        .get(entry_file_id)
        .expect("registered entry source must retain its identity")
        .logical_path;

    if !bundle_input {
        let outcome = {
            let (sources, mut span_builders) = source_builder.split();
            let source_code = sources
                .retained_text(entry_file_id)
                .expect("standalone entry text was retained immediately before preparation");
            let delta = prepare_template_source(
                sources,
                &request,
                source_code,
                entry_file_id,
                string_table,
                &mut path_fork,
                span_builders.take_span_builder(entry_file_id),
            );
            span_builders.retain_span_builder(entry_file_id, delta.span_builder);
            delta.result
        };
        let mut prepared = match outcome {
            Ok(prepared) => prepared,
            Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
                let FileFrontendPrepareError {
                    warnings,
                    diagnostic,
                    ..
                } = error;
                let mut diagnostics = preparation_warnings;
                for warning in warnings.into_diagnostics() {
                    diagnostics.push(warning);
                }
                diagnostics.push(diagnostic);
                let failure = PreparedFailure::capture_diagnosed(
                    diagnostics,
                    string_table.clone(),
                    None,
                    false,
                    None,
                );
                return Err(finish_source_owner(failure, source_builder));
            }
            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                let failure = prepared_infrastructure_failure_with_warnings(
                    error,
                    preparation_warnings,
                    string_table,
                );
                return Err(finish_source_owner(failure, source_builder));
            }
        };
        let freeze_result = prepared.freeze_path_syntax(string_table, &mut path_fork);
        for warning in prepared.warnings.take_diagnostics() {
            preparation_warnings.push(warning);
        }
        all_prepared.push(prepared);
        if let Err(error) = freeze_result {
            let failure = prepared_infrastructure_failure_with_warnings(
                error,
                preparation_warnings,
                string_table,
            );
            return Err(finish_source_owner(failure, source_builder));
        }
    }

    if let Some(error) = all_prepared
        .iter()
        .find_map(|prepared_source| prepared_source.require_frozen_path_syntax().err())
    {
        let failure = prepared_infrastructure_failure_with_warnings(
            error,
            preparation_warnings,
            string_table,
        );
        return Err(finish_source_owner(failure, source_builder));
    }

    let sorted = match order_template_headers(
        &mut all_prepared,
        &mut source_builder,
        &path_resolver,
        resolved_references.as_ref(),
        string_table,
        &mut path_fork,
    ) {
        Ok(sorted) => sorted,
        Err(failure) => {
            let failure = capture_prior_diagnostics_in_batch(
                failure,
                preparation_warnings,
                string_table,
                &path_fork,
            );
            return Err(finish_source_owner(failure, source_builder));
        }
    };

    let semantic_result = fold_template_semantics(
        sorted,
        entry_scope,
        &request,
        string_table,
        &mut path_fork,
        resolved_references,
        source_builder,
        module_origin,
        preparation_warnings,
    );

    match semantic_result {
        TemplateSemanticOutcome::Success {
            content,
            module_resources,
            warnings,
            source_builder,
        } => match source_builder.finish() {
            Ok(source_database) => Ok(FoldedMothTemplate {
                content,
                module_resources,
                warnings,
                source_database: Arc::new(source_database),
            }),
            Err(error) => {
                let failure = match warnings {
                    Some(batch) => PreparedFailure::Mixed {
                        batch,
                        error: error.into(),
                    },
                    None => PreparedFailure::from(error),
                };
                Err(failure.into())
            }
        },
        TemplateSemanticOutcome::Failure {
            failure,
            source_builder,
        } => Err(finish_source_owner(failure, source_builder)),
    }
}

enum TemplateSemanticOutcome {
    Success {
        content: OwnedFoldedString,
        module_resources: ModuleResourceTable,
        warnings: Option<PreparedDiagnosticBatch>,
        source_builder: SourceDatabaseBuilder,
    },
    Failure {
        failure: PreparedFailure,
        source_builder: SourceDatabaseBuilder,
    },
}

/// Bind and order retained syntax without consuming the preparation owner's span builders.
fn order_template_headers(
    prepared_sources: &mut [FileFrontendPrepareOutput],
    source_builder: &mut SourceDatabaseBuilder,
    path_resolver: &ProjectPathResolver,
    resolved_references: Option<&ResolvedFileReferenceTable>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<SortedHeaders, PreparedFailure> {
    let (source_files, _) = source_builder.split();
    let prepared_syntax = prepare_header_syntax(
        prepared_sources,
        string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
        path_fork,
    )
    .map_err(|failure| match failure {
        HeaderPreparationFailure::Diagnosed(bag) => {
            match PreparedDiagnosticBatch::capture_with_required_paths(
                bag,
                std::mem::take(string_table),
                None,
                || Some(Arc::new(path_fork.snapshot_table())),
            ) {
                Ok(batch) => PreparedFailure::Diagnosed(batch),
                Err(error) => PreparedFailure::from(error),
            }
        }
        HeaderPreparationFailure::Infrastructure(errors) => PreparedFailure::Infrastructure(errors),
    })?;
    let bound_headers = bind_module_headers(
        prepared_syntax,
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        &SourceProviderDependencySet::default(),
        Some(path_resolver),
        source_files,
        string_table,
        path_fork,
    )
    .map_err(|failure| match failure {
        HeaderPreparationFailure::Diagnosed(bag) => {
            match PreparedDiagnosticBatch::capture_with_required_paths(
                bag,
                std::mem::take(string_table),
                None,
                || Some(Arc::new(path_fork.snapshot_table())),
            ) {
                Ok(batch) => PreparedFailure::Diagnosed(batch),
                Err(error) => PreparedFailure::from(error),
            }
        }
        HeaderPreparationFailure::Infrastructure(errors) => PreparedFailure::Infrastructure(errors),
    })?;

    let content_source_targets = match resolved_references {
        Some(references) => ContentSourceTargets::from_resolved_references(
            references,
            source_files,
            string_table,
            path_fork,
        )
        .map_err(PreparedFailure::from)?,
        None => ContentSourceTargets::empty(),
    };
    resolve_module_dependencies(
        bound_headers,
        &content_source_targets,
        string_table,
        path_fork,
    )
}

fn capture_prior_diagnostics_in_batch(
    failure: PreparedFailure,
    prior_diagnostics: DiagnosticBag,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> PreparedFailure {
    let prior_diagnostics = prior_diagnostics.into_diagnostics();
    if prior_diagnostics.is_empty() {
        return failure;
    }

    match failure {
        PreparedFailure::Infrastructure(errors) => PreparedFailure::Mixed {
            batch: PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from_diagnostics(prior_diagnostics),
                string_table.clone(),
                None,
                false,
                None,
            ),
            error: errors,
        },
        mut failure => {
            if let Err(error) = failure
                .prepend_diagnostics_with_required_paths(prior_diagnostics, || {
                    Some(Arc::new(path_fork.snapshot_table()))
                })
            {
                PreparedFailure::from(error)
            } else {
                failure
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "template semantic folding keeps sorted headers, entry scope, the request, mutable string/path state, resolved references, the source builder, module origin, and prior diagnostics as separate inputs"
)]
fn fold_template_semantics(
    sorted: SortedHeaders,
    entry_scope: PathId,
    request: &MothTemplateCompilationRequest<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    resolved_references: Option<ResolvedFileReferenceTable>,
    source_builder: SourceDatabaseBuilder,
    module_origin: Option<StableModuleOriginIdentity>,
    prior_diagnostics: DiagnosticBag,
) -> TemplateSemanticOutcome {
    let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let frozen_identity_handle = module_origin
        .as_ref()
        .map(|origin| FrozenIdentityHandle::for_domain(origin.package().clone()))
        .unwrap_or_else(FrozenIdentityHandle::new);

    let source_database = source_builder.sources();
    let ast_result = {
        let file_value_resolution = resolved_references.map(|resolved_file_references| {
            Rc::new(FileValueResolutionServices {
                stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
                    resolved_file_references,
                    Arc::clone(source_database),
                ))),
                module_resources: Rc::clone(&module_resources),
                module_origin,
                frozen_identity_handle,
            })
        });
        fold_template_ast(
            sorted,
            entry_scope,
            request,
            string_table,
            path_fork,
            file_value_resolution,
        )
    };

    let mut ast = match ast_result {
        Ok(ast) => ast,
        Err(failure) => {
            return TemplateSemanticOutcome::Failure {
                failure: capture_prior_diagnostics_in_batch(
                    failure,
                    prior_diagnostics,
                    string_table,
                    path_fork,
                ),
                source_builder,
            };
        }
    };
    let mut warnings = prior_diagnostics;
    for warning in ast.warnings.drain(..) {
        warnings.push(warning);
    }
    let warning_diagnostics = warnings.into_diagnostics();
    let warning_batch = if warning_diagnostics.is_empty() {
        None
    } else {
        match PreparedDiagnosticBatch::capture_from_root_string_table(
            DiagnosticBag::from_diagnostics(warning_diagnostics),
            string_table,
            None,
            || Some(Arc::new(path_fork.snapshot_table())),
        ) {
            Ok(batch) => Some(batch),
            Err(error) => {
                return TemplateSemanticOutcome::Failure {
                    failure: PreparedFailure::from(error),
                    source_builder,
                };
            }
        }
    };

    let content = {
        let resources = module_resources.borrow();
        extract_content_value(&ast, entry_scope, &resources, path_fork, string_table)
    };
    drop(ast);

    let module_resources = match Rc::try_unwrap(module_resources) {
        Ok(resources) => resources.into_inner(),
        Err(_) => {
            return TemplateSemanticOutcome::Failure {
                failure: failure_with_warning_batch(
                    warning_batch,
                    CompilerError::compiler_error(
                        "Moth template resource table still has a live shared handle after AST folding",
                    ),
                ),
                source_builder,
            };
        }
    };

    match content {
        Ok(content) => TemplateSemanticOutcome::Success {
            content,
            module_resources,
            warnings: warning_batch,
            source_builder,
        },
        Err(error) => TemplateSemanticOutcome::Failure {
            failure: failure_with_warning_batch(warning_batch, error),
            source_builder,
        },
    }
}

fn failure_with_warning_batch(
    warnings: Option<PreparedDiagnosticBatch>,
    error: CompilerError,
) -> PreparedFailure {
    match warnings {
        Some(batch) => PreparedFailure::Mixed {
            batch,
            error: error.into(),
        },
        None => PreparedFailure::from(error),
    }
}

/// Prepare the standalone template source exactly once.
///
/// The preparation owner lends its split database and span-builder view for this one call, so
fn prepare_template_source(
    source_files: &Arc<SourceDatabase>,
    request: &MothTemplateCompilationRequest<'_>,
    source_code: &str,
    entry_file_id: SourceId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    span_builder: ExtendedSpanBuilder,
) -> SourcePreparationDelta {
    let options = HeaderParseOptions {
        entry_file_id: Some(entry_file_id),
        entry_file_role: None,
        active_root_role: ModuleRootRole::Normal,
    };
    let context = FrontendFilePrepareContext {
        source_files,
        style_directives: request.style_directives,
        entry_file_path: request.source_path,
        options: &options,
    };
    let input = FrontendFilePrepareInput {
        source: FrontendFilePrepareSource::MothTemplate { source_code },
        source_id: entry_file_id,
        span_builder,
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    CompilerFrontend::prepare_file_frontend_local(&context, input, string_table, path_fork)
}

fn fold_template_ast(
    sorted: SortedHeaders,
    entry_scope: PathId,
    request: &MothTemplateCompilationRequest<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    file_value_resolution: Option<Rc<FileValueResolutionServices>>,
) -> Result<Ast, PreparedFailure> {
    Ok(Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            source_token_owners: sorted.source_token_owners,
            module_symbols: sorted.module_symbols,
            binding_environment: sorted.binding_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
            source_build_config_contract_names: Arc::new(Default::default()),
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            style_directives: request.style_directives,
            string_table,
            path_fork,
            entry_dir: entry_scope,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution,
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
        },
    )?
    .ast)
}

/// Take the synthetic `content` constant every prepared `.mtf` source contributes.
///
/// The conversion is structural: resource pieces resolve through the folded module's resource
/// table, so a bundle-bearing fold keeps `Resource` and `SiteRoot` pieces intact. A resource
fn extract_content_value(
    ast: &Ast,
    entry_scope: PathId,
    resources: &ModuleResourceTable,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<OwnedFoldedString, CompilerError> {
    let content_path = content_constant_path(entry_scope, path_fork, string_table)?;
    let Some(content) = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| *row.path == content_path)
    else {
        return Err(CompilerError::compiler_error(
            "Moth template AST did not produce a content constant.",
        ));
    };

    ast.const_values
        .fold_value(content.id, &mut |_, visit| match visit {
            ConstValueVisit::String(value) => {
                owned_folded_string_from_const_string(value, resources, string_table)
            }
            ConstValueVisit::Template {
                folded: Some(value),
                ..
            } => owned_folded_string_from_const_string(value, resources, string_table),
            ConstValueVisit::Template { folded: None, .. } => Err(CompilerError::compiler_error(
                "Moth template content did not fold to a string.",
            )),
            _ => Err(CompilerError::compiler_error(
                "Moth template content did not fold to a string.",
            )),
        })
}

#[cfg(test)]
#[path = "tests/moth_template_tests.rs"]
mod tests;

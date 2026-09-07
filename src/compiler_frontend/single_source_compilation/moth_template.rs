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
//! the entry and content-source preparations, which this service consumes without preparing again
//! or probing the filesystem. Source collection, scope policy and output packaging stay with the caller.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::ast::const_values::store::ConstValueVisit;
use crate::compiler_frontend::ast::{
    Ast, AstBuildContext, AstBuildInput, FileValueResolutionServices, Stage0ResolutionFacts,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticBag};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure, FileFrontendPrepareOutput,
    HeaderParseOptions, bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::module_compilation::FrontendOptions;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, SortedHeaders, resolve_module_dependencies,
};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabase, SourceId, SourceKind, SourceRegistrationIndex,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
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
/// WHY:  physical file-reference resolution stays build-owned. The service consumes settled facts
///       and never probes the filesystem, while route and output placement stay out of the
///       compiler service entirely.
pub(crate) struct MothTemplateFileValueBundle {
    /// Prepared entry source with its final identity and frozen path syntax.
    pub(crate) prepared_entry: FileFrontendPrepareOutput,
    /// Prepared content dependencies with frozen path syntax, in discovery order.
    pub(crate) prepared_content_sources: Vec<FileFrontendPrepareOutput>,
    /// One settled outcome per prepared file-reference occurrence across the template and its
    /// content dependencies, keyed by `source_files` identities.
    pub(crate) resolved_file_references: ResolvedFileReferenceTable,
    /// Source identities of the template and all prepared content dependencies. The template's
    /// own canonical path must be present.
    pub(crate) source_files: SourceDatabase,
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
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    /// The finalized source snapshots and extended spans used by this template's diagnostics.
    pub(crate) source_database: Arc<SourceDatabase>,
}

/// Compile one Moth template source to its folded `content` value.
///
/// The service stops at folded AST data, so the projection side results of AST construction are
/// unused and no HIR, borrow, target validation or output stage runs.
pub(crate) fn compile_moth_template_source(
    request: MothTemplateCompilationRequest<'_>,
    string_table: &mut StringTable,
) -> Result<FoldedMothTemplate, CompilerMessages> {
    // This service has one source, so its own directory is both project and entry root and no
    // source package is reachable from it.
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
    let path_resolver = ProjectPathResolver::new(
        source_root.clone(),
        source_root,
        PreparedSourcePackageRoots::empty(),
        &source_file_kinds,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // A prepared bundle carries the module's Stage 0 identity facts; a plain request keeps one
    // self-contained in-memory source whose folds can never carry file values.
    let mut request = request;
    let mut direct_source_code = request.source_code.take();
    let file_value_resolution = request.file_value_resolution.take();
    let bundle_input = file_value_resolution.is_some();
    let (mut all_prepared, resolved_references, source_files, module_origin) =
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
                // A request without a Stage 0 bundle compiles one in-memory source. The one-row
                // inventory is registered here so this arm assigns `SourceId` through the same
                // canonical-order constructor as the bundle-bearing arm, which registered its
                // whole closure before this match.
                let registration_index = SourceRegistrationIndex::from_rows(std::iter::once((
                    request.source_path,
                    SourceKind::Compiler(SourceFileKind::MothTemplate),
                )));
                let mut source_files =
                    SourceDatabase::from_registration_index_sorted_by_logical_path(
                        &registration_index,
                        request.source_path,
                        Some(&path_resolver),
                        string_table,
                    )
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                let source_id = match source_files
                    .get_by_canonical_path(request.source_path)
                    .map(|identity| identity.id)
                {
                    Some(source_id) => source_id,
                    None => {
                        return Err(CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(
                                "standalone Moth template source identity was not registered",
                            ),
                            string_table,
                        ));
                    }
                };
                if let Some(source_code) = direct_source_code.take() {
                    source_files
                        .retain_text(source_id, source_code)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                }
                (Vec::new(), None, source_files, None)
            }
        };
    let source_files = Arc::new(source_files);

    let mut preparation_warnings = Vec::new();
    for prepared_source in &mut all_prepared {
        preparation_warnings.append(&mut prepared_source.warnings);
    }

    if bundle_input && direct_source_code.is_some() {
        let messages = CompilerMessages::from_error_with_warnings(
            CompilerError::compiler_error(
                "Moth template file-value bundle must own the retained source text",
            ),
            preparation_warnings,
            string_table,
        );
        return Err(attach_finalized_source_database(
            messages,
            source_files,
            all_prepared,
            string_table,
        ));
    }

    let source_identity = match source_files.get_by_canonical_path(request.source_path) {
        Some(source_identity) => source_identity,
        None => {
            let messages = CompilerMessages::from_error_with_warnings(
                CompilerError::compiler_error(
                    "Moth template source file table does not contain its own source",
                ),
                preparation_warnings,
                string_table,
            );
            return Err(attach_finalized_source_database(
                messages,
                source_files,
                all_prepared,
                string_table,
            ));
        }
    };
    let entry_file_id = source_identity.id;
    if all_prepared
        .first()
        .is_some_and(|prepared_entry| prepared_entry.file_id != entry_file_id)
    {
        let messages = CompilerMessages::from_error_with_warnings(
            CompilerError::compiler_error(
                "Moth template bundle entry does not match its registered source identity",
            ),
            preparation_warnings,
            string_table,
        );
        return Err(attach_finalized_source_database(
            messages,
            source_files,
            all_prepared,
            string_table,
        ));
    }
    let source_code = match source_files.retained_text(entry_file_id) {
        Some(source_code) => source_code,
        None => {
            let messages = CompilerMessages::from_error_with_warnings(
                CompilerError::compiler_error("Moth template source has no retained source text"),
                preparation_warnings,
                string_table,
            );
            return Err(attach_finalized_source_database(
                messages,
                source_files,
                all_prepared,
                string_table,
            ));
        }
    };
    let entry_scope = source_files.legacy_logical_path(entry_file_id);

    // Consume Stage 0's retained entry syntax, or prepare the standalone source once.
    if !bundle_input {
        let mut prepared = match prepare_template_source(
            &source_files,
            &path_resolver,
            &request,
            source_code,
            entry_file_id,
            string_table,
        ) {
            Ok(prepared) => prepared,
            Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
                return Err(diagnosed_preparation_messages(
                    source_files,
                    all_prepared,
                    error,
                    string_table,
                ));
            }
            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                let messages = CompilerMessages::from_error(error, string_table.clone());
                return Err(attach_finalized_source_database(
                    messages,
                    source_files,
                    all_prepared,
                    string_table,
                ));
            }
        };
        let freeze_result = prepared.freeze_path_syntax(string_table);
        preparation_warnings.append(&mut prepared.warnings);
        all_prepared.push(prepared);
        if let Err(error) = freeze_result {
            let messages = CompilerMessages::from_error_with_warnings(
                error,
                preparation_warnings,
                string_table,
            );
            return Err(attach_finalized_source_database(
                messages,
                source_files,
                all_prepared,
                string_table,
            ));
        }
    }

    // Bundle sources already passed whole-file validation before their final path tables froze.
    // Check that boundary without traversing all retained tokens and path rows again.
    if let Some(error) = all_prepared
        .iter()
        .find_map(|prepared_source| prepared_source.require_frozen_path_syntax().err())
    {
        let messages =
            CompilerMessages::from_error_with_warnings(error, preparation_warnings, string_table);
        return Err(attach_finalized_source_database(
            messages,
            source_files,
            all_prepared,
            string_table,
        ));
    }

    let sorted = match order_template_headers(
        &mut all_prepared,
        &source_files,
        &path_resolver,
        resolved_references.as_ref(),
        string_table,
    ) {
        Ok(sorted) => sorted,
        Err(bag) => {
            let mut messages =
                CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone());
            messages.prepend_diagnostics_preserving_context(preparation_warnings);
            return Err(attach_finalized_source_database(
                messages,
                source_files,
                all_prepared,
                string_table,
            ));
        }
    };

    // Folding releases all AST/service readers before the outcome owner installs the span tables.
    let semantic_result = fold_template_semantics(
        sorted,
        entry_scope,
        &request,
        string_table,
        resolved_references,
        source_files,
        module_origin,
    );

    match semantic_result {
        TemplateSemanticOutcome::Success {
            content,
            module_resources,
            mut warnings,
            source_database,
        } => {
            let mut source_database_warnings = preparation_warnings;
            source_database_warnings.append(&mut warnings);
            let source_database =
                match finalize_source_database(source_database, all_prepared, None) {
                    Ok(source_database) => source_database,
                    Err(error) => {
                        return Err(CompilerMessages::from_error_with_warnings(
                            error,
                            source_database_warnings,
                            string_table,
                        ));
                    }
                };
            Ok(FoldedMothTemplate {
                content,
                module_resources,
                warnings: source_database_warnings,
                source_database,
            })
        }

        TemplateSemanticOutcome::Diagnosed {
            mut messages,
            source_database,
        } => {
            messages.prepend_diagnostics_preserving_context(preparation_warnings);
            Err(attach_finalized_source_database(
                messages,
                source_database,
                all_prepared,
                string_table,
            ))
        }
    }
}

enum TemplateSemanticOutcome {
    Success {
        content: OwnedFoldedString,
        module_resources: ModuleResourceTable,
        warnings: Vec<CompilerDiagnostic>,
        source_database: Arc<SourceDatabase>,
    },
    Diagnosed {
        messages: CompilerMessages,
        source_database: Arc<SourceDatabase>,
    },
}

/// Bind and order retained syntax without consuming its source-local span builders.
fn order_template_headers(
    prepared_sources: &mut [FileFrontendPrepareOutput],
    source_files: &SourceDatabase,
    path_resolver: &ProjectPathResolver,
    resolved_references: Option<&ResolvedFileReferenceTable>,
    string_table: &mut StringTable,
) -> Result<SortedHeaders, DiagnosticBag> {
    let prepared_syntax = prepare_header_syntax(prepared_sources, string_table)?;
    let bound_headers = bind_module_headers(
        prepared_syntax,
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        &SourceProviderDependencySet::default(),
        Some(path_resolver),
        source_files,
        string_table,
    )?;

    // Stage 0 supplies content-source edges for bundles; standalone requests have none.
    let content_source_targets = match resolved_references {
        Some(references) => {
            ContentSourceTargets::from_resolved_references(references, source_files, string_table)
        }
        None => ContentSourceTargets::empty(),
    };
    resolve_module_dependencies(bound_headers, &content_source_targets, string_table)
}

fn fold_template_semantics(
    sorted: SortedHeaders,
    entry_scope: InternedPath,
    request: &MothTemplateCompilationRequest<'_>,
    string_table: &mut StringTable,
    resolved_references: Option<ResolvedFileReferenceTable>,
    source_database: Arc<SourceDatabase>,
    module_origin: Option<StableModuleOriginIdentity>,
) -> TemplateSemanticOutcome {
    let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));

    // This Arc is private to the service. All AST readers end before finalization needs exclusive
    // access, so installation can reuse the allocation without cloning the database.
    let ast_result = {
        let file_value_resolution = resolved_references.map(|resolved_file_references| {
            Rc::new(FileValueResolutionServices {
                stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
                    resolved_file_references,
                    Arc::clone(&source_database),
                ))),
                module_resources: Rc::clone(&module_resources),
                module_origin,
            })
        });
        fold_template_ast(
            sorted,
            entry_scope.clone(),
            request,
            string_table,
            file_value_resolution,
        )
    };

    let mut ast = match ast_result {
        Ok(ast) => ast,
        Err(messages) => {
            return TemplateSemanticOutcome::Diagnosed {
                messages,
                source_database,
            };
        }
    };
    let warnings = std::mem::take(&mut ast.warnings);

    // Release the resource-table borrow before consuming its owner.
    let content = {
        let resources = module_resources.borrow();
        extract_content_value(&ast, &entry_scope, &resources, string_table)
    };
    drop(ast);

    let module_resources = match Rc::try_unwrap(module_resources) {
        Ok(resources) => resources.into_inner(),
        Err(_) => {
            return TemplateSemanticOutcome::Diagnosed {
                messages: CompilerMessages::from_error_with_warnings(
                    CompilerError::compiler_error(
                        "Moth template resource table still has a live shared handle after AST folding",
                    ),
                    warnings,
                    string_table,
                ),
                source_database,
            };
        }
    };

    match content {
        Ok(content) => TemplateSemanticOutcome::Success {
            content,
            module_resources,
            warnings,
            source_database,
        },
        Err(mut messages) => {
            messages.prepend_diagnostics_preserving_context(warnings);
            TemplateSemanticOutcome::Diagnosed {
                messages,
                source_database,
            }
        }
    }
}

fn finalize_source_database(
    mut source_database: Arc<SourceDatabase>,
    prepared_sources: Vec<FileFrontendPrepareOutput>,
    failed_builder: Option<(SourceId, ExtendedSpanBuilder)>,
) -> Result<Arc<SourceDatabase>, CompilerError> {
    let sources = Arc::get_mut(&mut source_database).ok_or_else(|| {
        CompilerError::compiler_error(
            "Moth template source database remained shared after semantic folding",
        )
    })?;
    for prepared_source in prepared_sources {
        sources.install_extended_spans(
            prepared_source.file_id,
            prepared_source.span_builder.freeze(),
        )?;
    }
    if let Some((file_id, span_builder)) = failed_builder {
        sources.install_extended_spans(file_id, span_builder.freeze())?;
    }
    Ok(source_database)
}

fn attach_finalized_source_database(
    mut messages: CompilerMessages,
    source_database: Arc<SourceDatabase>,
    prepared_sources: Vec<FileFrontendPrepareOutput>,
    string_table: &StringTable,
) -> CompilerMessages {
    match finalize_source_database(source_database, prepared_sources, None) {
        Ok(source_database) => {
            messages.set_source_database(source_database);
            messages
        }
        Err(error) => CompilerMessages::from_error_with_warnings(
            error,
            messages.into_diagnostics(),
            string_table,
        ),
    }
}

fn diagnosed_preparation_messages(
    source_database: Arc<SourceDatabase>,
    prepared_sources: Vec<FileFrontendPrepareOutput>,
    error: FileFrontendPrepareError,
    string_table: &StringTable,
) -> CompilerMessages {
    let FileFrontendPrepareError {
        file_id,
        warnings,
        diagnostic,
        span_builder,
    } = error;
    let messages =
        CompilerMessages::from_diagnostic_with_warnings(*diagnostic, warnings, string_table);
    match finalize_source_database(
        source_database,
        prepared_sources,
        Some((file_id, span_builder)),
    ) {
        Ok(source_database) => {
            let mut messages = messages;
            messages.set_source_database(source_database);
            messages
        }
        Err(error) => CompilerMessages::from_error_with_warnings(
            error,
            messages.into_diagnostics(),
            string_table,
        ),
    }
}

fn prepare_template_source(
    source_files: &SourceDatabase,
    path_resolver: &ProjectPathResolver,
    request: &MothTemplateCompilationRequest<'_>,
    source_code: &str,
    entry_file_id: SourceId,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let options = HeaderParseOptions {
        entry_file_id: Some(entry_file_id),
        project_path_resolver: Some(path_resolver),
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
        source: FrontendFilePrepareSource::MothTemplate {
            source_code,
            source_path: request.source_path.to_path_buf(),
        },
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    CompilerFrontend::prepare_file_frontend_local(&context, input, string_table)
}

fn fold_template_ast(
    sorted: SortedHeaders,
    entry_scope: InternedPath,
    request: &MothTemplateCompilationRequest<'_>,
    string_table: &mut StringTable,
    file_value_resolution: Option<Rc<FileValueResolutionServices>>,
) -> Result<Ast, CompilerMessages> {
    let options = FrontendOptions::default();

    Ok(Ast::new(
        AstBuildInput {
            headers: sorted.headers,
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
            entry_dir: entry_scope,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution,
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit: options.template_const_loop_iteration_limit,
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
/// handle outside that table is a conversion invariant failure, never a user diagnostic.
fn extract_content_value(
    ast: &Ast,
    entry_scope: &InternedPath,
    resources: &ModuleResourceTable,
    string_table: &mut StringTable,
) -> Result<OwnedFoldedString, CompilerMessages> {
    let content_path = content_constant_path(entry_scope, string_table);
    let Some(content) = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path == &content_path)
    else {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error("Moth template AST did not produce a content constant."),
            string_table,
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
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

#[cfg(test)]
#[path = "tests/moth_template_tests.rs"]
mod tests;

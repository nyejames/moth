//! Direct Moth template compilation service.
//!
//! WHAT: the compiler-owned stage sequence for one `.mtf` source — tokenization, synthetic
//!       `content` header preparation, interface binding, local declaration ordering and AST
//!       folding — returning the folded `content` value, the folded module's resource-identity
//!       facts and the AST's warnings.
//!
//! WHY:  tooling needs template content without artifact planning, HIR, borrow validation or
//!       output writing. That shorter path is a named compiler service rather than a
//!       project-owned stage sequence: project code supplies one source and receives one folded
//!       result, and never prepares, binds, orders or folds the template itself.
//!
//! This is not a second Moth template parser or compiler mode. It uses the same owners as an
//! integrated `.mtf` dependency and must never grow a parallel Markdown or template renderer.
//! Physical file-reference resolution stays with the calling project: it supplies a prepared
//! file-value bundle and this service folds settled Stage 0 facts without probing the
//! filesystem. Source collection, scope policy and output packaging also stay with the caller.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::ast::const_values::store::ConstValueVisit;
use crate::compiler_frontend::ast::{
    Ast, AstBuildContext, AstBuildInput, FileValueResolutionServices, Stage0ResolutionFacts,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderParseOptions, bind_module_headers,
    prepare_header_syntax,
};
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::module_compilation::FrontendOptions;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
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
    /// Optional direct-request text. Bundle-bearing requests borrow their retained database text.
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
/// WHAT: the calling project's Stage 0 bundle for one template — its prepared content
///       dependencies, the settled physical outcome of every prepared file-reference occurrence,
///       and the source identities of the template with all its dependencies.
/// WHY:  physical file-reference resolution stays build-owned. The service consumes settled facts
///       and never probes the filesystem, while route and output placement stay out of the
///       compiler service entirely.
pub(crate) struct MothTemplateFileValueBundle {
    /// Prepared content dependencies named by the template's file values, in discovery order.
    pub(crate) prepared_content_sources: Vec<FileFrontendPrepareOutput>,
    /// One settled outcome per prepared file-reference occurrence across the template and its
    /// content dependencies, keyed by `source_files` identities.
    pub(crate) resolved_file_references: ResolvedFileReferenceTable,
    /// Source identities of the template and all prepared content dependencies. The template's
    /// own canonical path must be present.
    pub(crate) source_files: Arc<SourceDatabase>,
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
    let (mut prepared_content_sources, resolved_references, source_files, module_origin) =
        match file_value_resolution {
            Some(MothTemplateFileValueBundle {
                prepared_content_sources,
                resolved_file_references,
                source_files,
                module_origin,
            }) => {
                if direct_source_code.is_some() {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Moth template file-value bundle must own the retained source text",
                        ),
                        string_table,
                    ));
                }
                if source_files
                    .get_by_canonical_path(request.source_path)
                    .is_none()
                {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Moth template file-value bundle does not contain its own source",
                        ),
                        string_table,
                    ));
                }
                (
                    prepared_content_sources,
                    Some(resolved_file_references),
                    source_files,
                    module_origin,
                )
            }
            None => {
                let mut source_files = SourceDatabase::build(
                    [request.source_path],
                    request.source_path,
                    Some(&path_resolver),
                    string_table,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                let source_id = source_files
                    .get_by_canonical_path(request.source_path)
                    .map(|identity| identity.id)
                    .ok_or_else(|| {
                        CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(
                                "standalone Moth template source identity was not registered",
                            ),
                            string_table,
                        )
                    })?;
                if let Some(source_code) = direct_source_code.take() {
                    source_files
                        .retain_text(source_id, source_code)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                }
                (Vec::new(), None, Arc::new(source_files), None)
            }
        };
    let Some(source_identity) = source_files.get_by_canonical_path(request.source_path) else {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "Moth template source file table does not contain its own source",
            ),
            string_table,
        ));
    };
    let source_code = source_files
        .retained_text(source_identity.id)
        .ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error("Moth template source has no retained source text"),
                string_table,
            )
        })?;
    let entry_file_id = Some(source_identity.id);
    let entry_scope = source_identity.logical_path.clone();

    // 1. Prepare the single source into retained syntax.
    let mut prepared = prepare_template_source(
        &source_files,
        &path_resolver,
        &request,
        source_code,
        entry_file_id,
        string_table,
    )?;

    // This service has one final string domain and one final source per file. Freeze every
    // file-owned path table before header aggregation so AST parsing sees the same immutable
    // prepared-file contract as directory and synthetic module compilation.
    for content_source in &mut prepared_content_sources {
        content_source
            .freeze_path_syntax(string_table)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }
    prepared
        .freeze_path_syntax(string_table)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // 2. Aggregate retained syntax and bind it. A direct template resolves no provider.
    let mut all_prepared = Vec::with_capacity(1 + prepared_content_sources.len());
    all_prepared.push(prepared);
    all_prepared.extend(prepared_content_sources);
    let bound_headers = prepare_header_syntax(all_prepared, string_table)
        .and_then(|prepared_syntax| {
            bind_module_headers(
                prepared_syntax,
                &ExternalPackageRegistry::new(),
                &ExternalImportResolutionTable::default(),
                &SourceProviderDependencySet::default(),
                Some(&path_resolver),
                string_table,
            )
        })
        .map_err(|bag| {
            CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone())
        })?;

    // 3. Order local declarations. Content-source ordering edges come from Stage 0's resolved
    //    targets when a bundle is present and defer otherwise.
    let content_source_targets = match &resolved_references {
        Some(resolved_file_references) => ContentSourceTargets::from_resolved_references(
            resolved_file_references,
            &source_files,
            string_table,
        ),
        None => ContentSourceTargets::empty(),
    };
    let sorted = resolve_module_dependencies(bound_headers, &content_source_targets, string_table)
        .map_err(|bag| {
            CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone())
        })?;

    // 4. Fold the ordered declarations and take the synthetic `content` constant. The bundle's
    //    resolved rows and module origin drive AST value semantics exactly like an integrated
    //    module fold; resource pieces intern into the table retained here.
    let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let file_value_resolution = resolved_references.map(|resolved_file_references| {
        Rc::new(FileValueResolutionServices {
            stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
                resolved_file_references,
                Arc::clone(&source_files),
            ))),
            module_resources: Rc::clone(&module_resources),
            module_origin,
        })
    });
    let mut ast = fold_template_ast(
        sorted,
        entry_scope.clone(),
        &request,
        string_table,
        file_value_resolution,
    )?;
    let warnings = std::mem::take(&mut ast.warnings);

    // Bind the conversion result first: the read borrow must release before the source facts are
    // moved out for the calling project.
    let content =
        extract_content_value(&ast, &entry_scope, &module_resources.borrow(), string_table);

    match content {
        Ok(content) => {
            // Folding is complete and no fold stage holds the table's value, so the service moves
            // its source facts out for the calling project.
            let module_resources = std::mem::take(&mut *module_resources.borrow_mut());
            Ok(FoldedMothTemplate {
                content,
                module_resources,
                warnings,
            })
        }
        Err(mut messages) => {
            messages.prepend_diagnostics_preserving_context(warnings);
            Err(messages)
        }
    }
}

fn prepare_template_source(
    source_files: &SourceDatabase,
    path_resolver: &ProjectPathResolver,
    request: &MothTemplateCompilationRequest<'_>,
    source_code: &str,
    entry_file_id: Option<SourceId>,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, CompilerMessages> {
    let options = HeaderParseOptions {
        entry_file_id,
        project_path_resolver: Some(path_resolver.clone()),
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

fn fold_template_ast(
    sorted: crate::compiler_frontend::module_dependencies::SortedHeaders,
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

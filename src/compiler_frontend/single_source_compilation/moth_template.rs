//! Direct Moth template compilation service.
//!
//! WHAT: the compiler-owned stage sequence for one `.mtf` source — tokenization, synthetic
//!       `content` header preparation, interface binding, local declaration ordering and AST
//!       folding — returning the folded `content` value and the AST's warnings.
//!
//! WHY:  tooling needs template content as a string without artifact planning, HIR, borrow
//!       validation or output writing. That shorter path is a named compiler service rather than a
//!       project-owned stage sequence: project code supplies one source and receives one folded
//!       result, and never prepares, binds, orders or folds the template itself.
//!
//! This is not a second Moth template parser or compiler mode. It uses the same owners as an
//! integrated `.mtf` dependency and must never grow a parallel Markdown or template renderer.
//! Source collection, scope policy and output packaging stay with the calling project.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::ast::const_values::store::ConstValueVisit;
use crate::compiler_frontend::ast::{Ast, AstBuildContext, AstBuildInput};
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
use crate::compiler_frontend::module_compilation::FrontendOptions;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendBuildProfile, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One Moth template source and the style vocabulary it folds against.
pub(crate) struct MothTemplateCompilationRequest<'a> {
    /// The canonical path of the template source, used for its file identity.
    pub(crate) source_path: &'a Path,
    pub(crate) source_code: String,
    /// The calling project's style directives, already merged.
    ///
    /// WHY: which directives a template may use is project vocabulary, so the caller owns the
    ///      registry and the service never assembles one from a project.
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
}

/// The folded template a caller packages into its own output shape.
///
/// The content keeps resource and site-root anchors opaque until a builder supplies its link plan.
/// Plain text remains a dedicated `Text` value, so it does not allocate a piece vector.
pub(crate) struct FoldedMothTemplate {
    pub(crate) content: OwnedFoldedString,
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

    let source_files = SourceFileTable::build(
        [request.source_path],
        request.source_path,
        Some(&path_resolver),
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let Some(source_identity) = source_files.get_by_canonical_path(request.source_path) else {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "Moth template source file table does not contain its own source",
            ),
            string_table,
        ));
    };
    let entry_file_id = Some(source_identity.file_id);
    let entry_scope = source_identity.logical_path.clone();

    // 1. Prepare the single source into retained syntax.
    let mut prepared = prepare_template_source(
        &source_files,
        &path_resolver,
        &request,
        entry_file_id,
        string_table,
    )?;

    // This service has one final source and one final string domain. Freeze its file-owned path
    // table before header aggregation so AST parsing sees the same immutable prepared-file
    // contract as directory and synthetic module compilation.
    prepared
        .freeze_path_syntax(string_table)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // 2. Aggregate retained syntax and bind it. A direct template resolves no provider.
    let bound_headers = prepare_header_syntax(vec![prepared], string_table)
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

    // 3. Order local declarations.
    let sorted =
        resolve_module_dependencies(bound_headers, &ContentSourceTargets::empty(), string_table)
            .map_err(|bag| {
                CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone())
            })?;

    // 4. Fold the ordered declarations and take the synthetic `content` constant.
    let mut ast = fold_template_ast(sorted, entry_scope, &request, string_table)?;
    let warnings = std::mem::take(&mut ast.warnings);

    match extract_content_value(&ast, string_table) {
        Ok(content) => Ok(FoldedMothTemplate { content, warnings }),
        Err(mut messages) => {
            messages.prepend_diagnostics_preserving_context(warnings);
            Err(messages)
        }
    }
}

fn prepare_template_source(
    source_files: &SourceFileTable,
    path_resolver: &ProjectPathResolver,
    request: &MothTemplateCompilationRequest<'_>,
    entry_file_id: Option<crate::compiler_frontend::symbols::identity::FileId>,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, CompilerMessages> {
    let options = HeaderParseOptions {
        entry_file_id,
        project_path_resolver: Some(path_resolver.clone()),
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
            source_code: request.source_code.clone(),
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
) -> Result<Ast, CompilerMessages> {
    let options = FrontendOptions::default();

    Ok(Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            module_symbols: sorted.module_symbols,
            binding_environment: sorted.binding_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            style_directives: request.style_directives,
            string_table,
            entry_dir: entry_scope,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution: None,
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
/// The direct service deliberately has no file-value resolution services, so the resource table is
/// empty today. Keeping the conversion structural here means Phase 5 can supply the normal
/// prepared dependency bundle without changing this result boundary.
fn extract_content_value(
    ast: &Ast,
    string_table: &StringTable,
) -> Result<OwnedFoldedString, CompilerMessages> {
    let Some(content) = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(string_table) == Some("content"))
    else {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error("Moth template AST did not produce a content constant."),
            string_table,
        ));
    };

    // WHY: this direct `.mtf` lane carries no file-value resolution services, so its folded
    //      content cannot contain `Resource` pieces and an empty table is sound, not a
    //      placeholder. If one reaches this site anyway, the conversion's `CompilerError`
    //      is the invariant failure; never substitute an empty or default origin.
    let resources = ModuleResourceTable::new();
    ast.const_values
        .fold_value(content.id, &mut |_, visit| match visit {
            ConstValueVisit::String(value) => {
                owned_folded_string_from_const_string(value, &resources, string_table)
            }
            ConstValueVisit::Template {
                folded: Some(value),
                ..
            } => owned_folded_string_from_const_string(value, &resources, string_table),
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

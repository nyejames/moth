//! Direct Moth template compile orchestration.
//!
//! WHAT: runs each Moth template source through frontend preparation, header aggregation, dependency
//! sorting, and AST folding, then extracts the synthetic `content` constant.
//! WHY: this gives HTML tooling a string-producing API while preserving the compiler's stage
//! boundaries and avoiding HIR generation, borrow validation, artifact writing, or a duplicate
//! Markdown/template renderer.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::{
    HeaderParseOptions, bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::module_dependencies::SortedHeaders;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendBuildProfile, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::projects::html_project::moth_template::input::{
    MothTemplateCompileRequest, MothTemplateSourceUnit,
};
use crate::projects::html_project::moth_template::output::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput,
};
use crate::projects::html_project::style_directives::html_project_style_directives;
use crate::projects::settings::Config;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) fn compile_moth_template(
    request: MothTemplateCompileRequest,
    string_table: &mut StringTable,
) -> Result<MothTemplateCompileOutput, CompilerMessages> {
    let sources = request.collect_sources(string_table)?;
    let mut documents = Vec::with_capacity(sources.len());
    let mut warnings = Vec::new();

    for source in sources {
        match compile_one_source(source, string_table, &mut warnings) {
            Ok(document) => documents.push(document),
            Err(mut messages) => {
                messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
                return Err(messages);
            }
        }
    }

    Ok(MothTemplateCompileOutput {
        documents,
        warnings,
    })
}

fn compile_one_source(
    source: MothTemplateSourceUnit,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
) -> Result<CompiledMothTemplateDocument, CompilerMessages> {
    let mut compiler =
        new_direct_moth_template_frontend(&source.source_path, string_table.clone())?;

    let source_files = SourceFileTable::build(
        [source.source_path.as_path()],
        source.source_path.as_path(),
        compiler.project_path_resolver.as_ref(),
        &mut compiler.string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    compiler.set_source_files(source_files);

    let prepared = prepare_source_file(&mut compiler, &source)?;
    let prepared_syntax = prepare_header_syntax(vec![prepared], &mut compiler.string_table)
        .map_err(|bag| {
            CompilerMessages::from_diagnostics(
                bag.into_diagnostics(),
                compiler.string_table.clone(),
            )
        })?;

    let headers = bind_module_headers(
        prepared_syntax,
        compiler.external_package_registry.as_ref(),
        &ExternalImportResolutionTable::default(),
        compiler.project_path_resolver.as_ref(),
        &mut compiler.string_table,
    )
    .map_err(|bag| {
        CompilerMessages::from_diagnostics(bag.into_diagnostics(), compiler.string_table.clone())
    })?;

    let sorted = sort_headers(&mut compiler, headers)?;
    let ast = compiler.headers_to_ast(
        sorted,
        &source.source_path,
        FrontendBuildProfile::Dev,
        Default::default(),
    )?;
    warnings.extend(ast.warnings.clone());

    let content = extract_content_string(&ast.module_constants, &compiler.string_table)?;
    *string_table = compiler.string_table;

    Ok(CompiledMothTemplateDocument {
        source_path: source.source_path,
        relative_path: source.relative_path,
        content,
    })
}

fn new_direct_moth_template_frontend(
    source_path: &Path,
    string_table: StringTable,
) -> Result<CompilerFrontend, CompilerMessages> {
    let source_root = source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut source_file_kinds = SourceFileKindRegistry::new();
    source_file_kinds.register(
        SourceFileKind::MothTemplate.extension(),
        SourceFileKind::MothTemplate,
    );
    let project_path_resolver = ProjectPathResolver::new(
        source_root.clone(),
        source_root,
        PreparedSourcePackageRoots::empty(),
        &source_file_kinds,
    )
    .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    Ok(CompilerFrontend::new(
        &Config::default(),
        string_table,
        style_directives,
        Arc::new(ExternalPackageRegistry::new()),
        Some(project_path_resolver),
    ))
}

fn prepare_source_file(
    compiler: &mut CompilerFrontend,
    source: &MothTemplateSourceUnit,
) -> Result<
    crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareOutput,
    CompilerMessages,
> {
    let options = HeaderParseOptions {
        entry_file_id: compiler
            .source_files
            .get_by_canonical_path(&source.source_path)
            .map(|identity| identity.file_id),
        project_path_resolver: compiler.project_path_resolver.clone(),
    };
    let context = FrontendFilePrepareContext {
        source_files: &compiler.source_files,
        style_directives: &compiler.style_directives,
        entry_file_path: source.source_path.as_path(),
        options: &options,
    };
    let input = FrontendFilePrepareInput {
        source: FrontendFilePrepareSource::MothTemplate {
            source_code: &source.source_text,
            source_path: &source.source_path,
        },
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    CompilerFrontend::prepare_file_frontend_local(&context, input, &mut compiler.string_table)
        .map_err(|mut error| {
            let mut messages =
                CompilerMessages::from_diagnostic(*error.diagnostic, compiler.string_table.clone());
            messages.prepend_diagnostics_preserving_context(error.warnings.drain(..));
            messages
        })
}

fn sort_headers(
    compiler: &mut CompilerFrontend,
    headers: crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
) -> Result<SortedHeaders, CompilerMessages> {
    compiler.sort_headers(headers).map_err(|bag| {
        CompilerMessages::from_diagnostics(bag.into_diagnostics(), compiler.string_table.clone())
    })
}

fn extract_content_string(
    module_constants: &[crate::compiler_frontend::ast::ast_nodes::Declaration],
    string_table: &StringTable,
) -> Result<String, CompilerMessages> {
    let Some(content) = module_constants
        .iter()
        .find(|constant| constant.id.name_str(string_table) == Some("content"))
    else {
        return Err(CompilerMessages::from_error(
            CompilerError::compiler_error("Moth template AST did not produce a content constant."),
            string_table.clone(),
        ));
    };

    let ExpressionKind::StringSlice(value) = &content.value.kind else {
        return Err(CompilerMessages::from_error(
            CompilerError::compiler_error("Moth template content did not fold to a string."),
            string_table.clone(),
        ));
    };

    Ok(string_table.resolve(*value).to_owned())
}

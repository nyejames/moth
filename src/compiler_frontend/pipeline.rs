//! The frontend stage facade.
//!
//! WHAT: one value carrying the registries, options, string table and file identities every stage
//!       needs, plus the thin per-stage calls that read them.
//! WHY:  the stage owners each need a different slice of the same immutable context. Holding it
//!       once keeps a service's flow readable as named stage calls instead of a growing argument
//!       list threaded through each one.
//!
//! This is not a production entry point. The services in `module_compilation` and
//! `single_source_compilation` decide which stages run and in what order.
//!
//! Visibility follows that split. Tokenization and per-file preparation are crate-visible because
//! Stage 0 legitimately prepares source before any module is ready. Every semantic stage —
//! ordering, AST, HIR and borrow validation — is visible only inside `compiler_frontend`, so build
//! or project code cannot assemble a semantic sequence of its own.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, check_borrows as run_borrow_checker,
};
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::{
    Ast, AstBuildContext, AstBuildInput, AstBuildResult, FileValueResolutionServices,
    Stage0ResolutionFacts,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::moth_template_prepare::prepare_moth_template_file;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, FileFrontendPrepareError, FileFrontendPrepareFailure,
    FileFrontendPrepareOutput, HeaderParseOptions, parse_file_headers_with_table,
};
use crate::compiler_frontend::headers::plain_markdown_prepare::{
    PlainMarkdownPrepareInput, prepare_plain_markdown_file,
};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::hir_builder::lower_module;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_compilation::FrontendOptions;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, SortedHeaders, resolve_module_dependencies,
};
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenizerEntryMode};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{LazyLock, Mutex};

#[cfg(test)]
static FILE_FRONTEND_PREPARE_COUNTS_FOR_TEST: LazyLock<Mutex<HashMap<PathBuf, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
#[cfg(test)]
static FILE_FRONTEND_PREPARE_TRACK_PREFIX_FOR_TEST: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
fn record_file_frontend_prepare_for_test(source: &FrontendFilePrepareSource) {
    let source_path = match source {
        FrontendFilePrepareSource::Moth { source_path, .. }
        | FrontendFilePrepareSource::MothTemplate { source_path, .. }
        | FrontendFilePrepareSource::PlainMarkdown { source_path, .. } => source_path,
    };
    let prefix = FILE_FRONTEND_PREPARE_TRACK_PREFIX_FOR_TEST
        .lock()
        .expect("file preparation test hook lock poisoned");
    if prefix
        .as_ref()
        .is_none_or(|tracked_prefix| source_path.starts_with(tracked_prefix))
    {
        *FILE_FRONTEND_PREPARE_COUNTS_FOR_TEST
            .lock()
            .expect("file preparation count test hook lock poisoned")
            .entry(source_path.clone())
            .or_insert(0) += 1;
    }
}

#[cfg(test)]
pub(crate) fn reset_file_frontend_prepare_count_for_test(tracked_prefix: &Path) {
    *FILE_FRONTEND_PREPARE_TRACK_PREFIX_FOR_TEST
        .lock()
        .expect("file preparation test hook lock poisoned") = Some(tracked_prefix.to_path_buf());
    FILE_FRONTEND_PREPARE_COUNTS_FOR_TEST
        .lock()
        .expect("file preparation count test hook lock poisoned")
        .clear();
}

#[cfg(test)]
pub(crate) fn file_frontend_prepare_count_for_path_for_test(path: &Path) -> usize {
    FILE_FRONTEND_PREPARE_COUNTS_FOR_TEST
        .lock()
        .expect("file preparation count test hook lock poisoned")
        .get(path)
        .copied()
        .unwrap_or(0)
}

pub(crate) struct CompilerFrontend {
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) string_table: StringTable,
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) options: FrontendOptions,
    /// Immutable source identities registered once by the enclosing compilation boundary and
    /// shared, never copied, by every module compiled inside it.
    pub(crate) source_files: Arc<SourceDatabase>,
}

/// Shared immutable inputs used while one source file is prepared against a local string table.
///
/// WHAT: collects the frontend-owned registries and entry-file identity needed by tokenization and
/// header parsing.
/// WHY: parallel file preparation passes this context by shared reference to Rayon workers without
/// giving them mutable access to the module-global string table.
pub(crate) struct FrontendFilePrepareContext<'a> {
    pub(crate) source_files: &'a SourceDatabase,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) entry_file_path: &'a Path,
    pub(crate) options: &'a HeaderParseOptions,
}

/// Owned per-file source payload for frontend preparation.
///
/// WHAT: one variant per source kind. Moth carries the retained `FileTokens` from the
///       single Stage 0 lexical pass; Moth template and PlainMarkdown carry only raw source text.
/// WHY: the variant makes the source-kind/token relationship unrepresentable as an invalid
///      state. The Moth preparation arm receives `FileTokens` by type, so it cannot panic
///      on absent tokens, and Moth template/PlainMarkdown cannot carry Moth tokens.
///
/// The build system moves its source-kind handoff into this value; the frontend does not depend on
/// build-system types and owns each payload for the duration of header preparation.
pub(crate) enum FrontendFilePrepareSource {
    Moth {
        source_path: PathBuf,
        tokens: Box<FileTokens>,
    },
    MothTemplate {
        source_code: String,
        source_path: PathBuf,
    },
    PlainMarkdown {
        source_code: String,
        source_path: PathBuf,
    },
}

/// Per-file source payload and numbering offsets for local frontend preparation.
///
/// WHAT: keeps the state-safe source variant and synthetic-fragment offsets together for one
///       worker item.
/// WHY: grouping these inputs keeps the preparation API explicit without a broad argument list.
pub(crate) struct FrontendFilePrepareInput {
    pub(crate) source: FrontendFilePrepareSource,
    pub(crate) const_template_offset: usize,
    pub(crate) runtime_fragment_offset: usize,
}

/// Everything `headers_to_ast` needs to build one module's AST.
///
/// WHAT: the sorted header set, entry identity, build profile, arena estimate and the Stage 0
///       file-value facts AST interprets for value-position paths.
/// WHY: the stage outgrew a readable positional list once Stage 0 resolution joined it, and a
///      named group keeps every call site self-describing instead of eight bare positions.
pub(crate) struct AstBuildRequest<'a> {
    pub(crate) sorted: SortedHeaders,
    pub(crate) entry_file_path: &'a Path,
    pub(crate) root_role: ModuleRootRole,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) capacity_estimate: FrontendArenaCapacityEstimate,
    pub(crate) resolved_file_references: ResolvedFileReferenceTable,
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
    pub(crate) build_config_values:
        Arc<crate::compiler_frontend::build_config::ResolvedBuildConfigMap>,
}

/// Stable identity facts for one source file as seen by the frontend.
///
/// WHAT: bundles the interned logical path, explicit file ID, and canonical OS path that
///       tokenization and non-tokenized preparation both need.
/// WHY: keeps source-identity lookup in one place so Markdown preparation can reuse the same
///      identity path as tokenized files without duplicating the `SourceDatabase` fallback logic.
struct FrontendSourceFileIdentity {
    logical_path: InternedPath,
    file_id: Option<SourceId>,
    canonical_os_path: Option<PathBuf>,
}

/// Look up frontend identity for a source path.
///
/// WHAT: returns the logical interned path, stable file ID, and canonical OS path for one file.
/// WHY: tokenized Moth/Moth template files and non-tokenized Markdown files must share the same
///      source identity so downstream stages treat them as ordinary module members.
fn source_file_identity(
    source_files: &SourceDatabase,
    source_path: &Path,
    string_table: &mut StringTable,
) -> Result<FrontendSourceFileIdentity, CompilerError> {
    match source_files.get_by_canonical_path(source_path) {
        Some(identity) => Ok(FrontendSourceFileIdentity {
            logical_path: identity.logical_path.clone(),
            file_id: Some(identity.id),
            canonical_os_path: Some(identity.canonical_os_path.clone()),
        }),
        None => {
            let logical_path =
                InternedPath::try_from_filesystem_path(source_path, string_table).map_err(
                    |NonUtf8PathComponent { path }| {
                        CompilerError::file_error(
                            &path,
                            format!(
                                "Source file path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                            ),
                            string_table,
                        )
                    },
                )?;
            Ok(FrontendSourceFileIdentity {
                logical_path,
                file_id: None,
                canonical_os_path: Some(source_path.to_owned()),
            })
        }
    }
}

impl CompilerFrontend {
    pub(crate) fn new(
        options: FrontendOptions,
        string_table: StringTable,
        style_directives: StyleDirectiveRegistry,
        external_package_registry: Arc<ExternalPackageRegistry>,
        project_path_resolver: Option<ProjectPathResolver>,
        source_files: Arc<SourceDatabase>,
    ) -> Self {
        Self {
            external_package_registry,
            style_directives,
            string_table,
            project_path_resolver,
            options,
            source_files,
        }
    }

    // -----------------------------
    //  TOKENIZER
    // -----------------------------
    /// Tokenize source text against an explicitly supplied string table.
    ///
    /// WHAT: resolves source file identity and runs tokenization without assuming ownership of the
    ///       string table. This allows per-file tokenization against local string-table forks.
    /// WHY: parallel and fork-based frontend preparation need to tokenize independently before
    ///      merging deltas back into the module/global table.
    pub(crate) fn tokenize_source(
        source_files: &SourceDatabase,
        style_directives: &StyleDirectiveRegistry,
        source_code: &str,
        module_path: &Path,
        tokenizer_entry_mode: TokenizerEntryMode,
        string_table: &mut StringTable,
    ) -> Result<FileTokens, FileFrontendPrepareFailure> {
        let identity = source_file_identity(source_files, module_path, string_table)
            .map_err(FileFrontendPrepareFailure::Infrastructure)?;

        let mut tokens = tokenize(
            source_code,
            &identity.logical_path,
            tokenizer_entry_mode,
            style_directives,
            string_table,
            identity.file_id,
        )
        .map_err(|diagnostic| {
            FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                warnings: Vec::new(),
                diagnostic,
            })
        })?;
        tokens.canonical_os_path = identity.canonical_os_path;
        Ok(tokens)
    }

    /// Prepare one source file against a caller-provided local string table.
    ///
    /// WHAT: parses retained Moth tokens, tokenizes and prepares Moth template, or prepares plain
    ///       Markdown without merge/remap so callers can run file work in parallel.
    /// WHY: parallel frontend preparation needs each worker to own its local table without shared
    ///      mutable access to the module-global table, while Stage 0 remains the sole tokenizer
    ///      owner for discovered Moth source.
    pub(crate) fn prepare_file_frontend_local(
        context: &FrontendFilePrepareContext<'_>,
        input: FrontendFilePrepareInput,
        local_string_table: &mut StringTable,
    ) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
        add_frontend_counter(FrontendCounter::FilePreparationPassCount, 1);
        #[cfg(test)]
        record_file_frontend_prepare_for_test(&input.source);

        match input.source {
            FrontendFilePrepareSource::PlainMarkdown {
                source_code,
                source_path,
            } => {
                let identity =
                    source_file_identity(context.source_files, &source_path, local_string_table)
                        .map_err(FileFrontendPrepareFailure::Infrastructure)?;
                Ok(prepare_plain_markdown_file(
                    PlainMarkdownPrepareInput {
                        source_code: &source_code,
                        source_file: identity.logical_path,
                        file_id: identity.file_id,
                        canonical_os_path: identity.canonical_os_path,
                    },
                    local_string_table,
                ))
            }
            FrontendFilePrepareSource::Moth {
                source_path,
                mut tokens,
            } => {
                // Moth files carry the exact token stream retained from the single Stage 0
                // lexical pass. Rebind it to the module source identity and parse headers without
                // re-tokenizing. `tokens` is present by type, so no absent-token panic is possible.
                let identity =
                    source_file_identity(context.source_files, &source_path, local_string_table)
                        .map_err(FileFrontendPrepareFailure::Infrastructure)?;

                tokens
                    .rebind_source_identity(
                        identity.logical_path,
                        identity.file_id,
                        identity.canonical_os_path,
                    )
                    .map_err(FileFrontendPrepareFailure::Infrastructure)?;

                parse_file_headers_with_table(
                    &mut tokens,
                    context.entry_file_path,
                    context.options,
                    local_string_table,
                    input.const_template_offset,
                    input.runtime_fragment_offset,
                )
            }
            FrontendFilePrepareSource::MothTemplate {
                source_code,
                source_path,
            } => {
                // Moth template is tokenized exactly once by its template-body preparation path.
                let tokenizer_entry_mode =
                    match TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate) {
                        Some(mode) => mode,
                        None => unreachable!("Moth template has a tokenizer entry mode"),
                    };

                let file_tokens = Self::tokenize_source(
                    context.source_files,
                    context.style_directives,
                    &source_code,
                    &source_path,
                    tokenizer_entry_mode,
                    local_string_table,
                )?;

                prepare_moth_template_file(file_tokens, local_string_table)
                    .map_err(FileFrontendPrepareFailure::Infrastructure)
            }
        }
    }

    // ---------------------------
    //  DEPENDENCY SORTING
    // ---------------------------
    pub(in crate::compiler_frontend) fn sort_headers(
        &mut self,
        headers: BoundModuleHeaders,
        resolved_file_references: &ResolvedFileReferenceTable,
    ) -> Result<SortedHeaders, DiagnosticBag> {
        // Content-source ordering edges resolve through Stage 0's canonical targets, which this
        // compiler instance already retains as the module source identities.
        let content_source_targets = ContentSourceTargets::from_resolved_references(
            resolved_file_references,
            &self.source_files,
            &mut self.string_table,
        );

        resolve_module_dependencies(headers, &content_source_targets, &mut self.string_table)
    }

    pub(in crate::compiler_frontend) fn headers_to_ast(
        &mut self,
        request: AstBuildRequest<'_>,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<AstBuildResult, CompilerMessages> {
        let AstBuildRequest {
            sorted,
            entry_file_path,
            root_role,
            build_profile,
            capacity_estimate,
            resolved_file_references,
            module_origin,
            build_config_values,
        } = request;

        let interned_entry_file = match self.source_files.get_by_canonical_path(entry_file_path) {
            Some(identity) => identity.logical_path.clone(),
            None => match InternedPath::try_from_filesystem_path(
                entry_file_path,
                &mut self.string_table,
            ) {
                Ok(path) => path,
                Err(NonUtf8PathComponent { path }) => {
                    let error = CompilerError::file_error(
                        &path,
                        format!(
                            "Entry file path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                        ),
                        &mut self.string_table,
                    );
                    return Err(CompilerMessages::from_error_ref(error, &self.string_table));
                }
            },
        };

        let file_value_resolution = Some(Rc::new(FileValueResolutionServices {
            stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
                resolved_file_references,
                Arc::clone(&self.source_files),
            ))),
            module_resources: Rc::new(RefCell::new(ModuleResourceTable::new())),
            module_origin,
        }));
        Ast::new(
            AstBuildInput {
                source_build_config_contract_names: Arc::new(
                    sorted
                        .source_build_config_contracts
                        .iter()
                        .map(|contract| contract.name.clone())
                        .collect(),
                ),
                headers: sorted.headers,
                module_symbols: sorted.module_symbols,
                binding_environment: sorted.binding_environment,
                top_level_const_fragments: sorted.top_level_const_fragments,
            },
            AstBuildContext {
                external_package_registry: Arc::clone(&self.external_package_registry),
                style_directives: &self.style_directives,
                string_table: &mut self.string_table,
                entry_dir: interned_entry_file,
                root_role,
                build_profile,
                file_value_resolution,
                config_resolution: None,
                build_config_values,
                template_const_loop_iteration_limit: self
                    .options
                    .template_const_loop_iteration_limit,
                capacity_estimate,
                #[cfg(feature = "timers")]
                timing_context,
                #[cfg(feature = "timers")]
                timing_metric_family:
                    crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
            },
        )
    }

    // -----------------------------
    //  HIR GENERATION
    // -----------------------------
    pub(in crate::compiler_frontend) fn generate_hir(
        &mut self,
        ast: Ast,
        function_origin_lookup: HirFunctionOriginLookup,
        module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    ) -> Result<HirLoweringResult, CompilerMessages> {
        let static_if_function_provenance = ast.static_if_function_provenance.clone();
        let mut result = lower_module(
            ast,
            &mut self.string_table,
            function_origin_lookup,
            module_resources,
        )?;
        for (function_path, provenance) in static_if_function_provenance {
            let Some(function_id) = result.hir_module.functions.iter().find_map(|function| {
                (result.hir_module.side_table.function_name_path(function.id)
                    == Some(&function_path))
                .then_some(function.id)
            }) else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Static configuration provenance has no lowered HIR function for path {:?}",
                        function_path
                    )),
                    &self.string_table,
                ));
            };
            let Some(existing_provenance) =
                result.hir_module.function_provenance.get_mut(&function_id)
            else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "HIR function {:?} is missing its provenance fact while applying static configuration provenance",
                        function_id
                    )),
                    &self.string_table,
                ));
            };
            existing_provenance.merge(&provenance);
        }
        Ok(result)
    }

    // ------------------------------
    //  BORROW CHECKING AND ANALYSIS
    // ------------------------------
    pub(in crate::compiler_frontend) fn check_borrows(
        &self,
        hir_module: &HirModule,
    ) -> Result<BorrowCheckReport, CompilerMessages> {
        match run_borrow_checker(
            hir_module,
            self.external_package_registry.as_ref(),
            &self.string_table,
        ) {
            Ok(report) => Ok(report),
            Err(error) => match error.into_diagnostic_or_infrastructure() {
                Ok(diagnostic) => Err(CompilerMessages::from_diagnostic_ref(
                    diagnostic,
                    &self.string_table,
                )),
                Err(error) => Err(CompilerMessages::from_error_ref(error, &self.string_table)),
            },
        }
    }
}

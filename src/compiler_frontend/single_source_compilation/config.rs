//! Project config compilation service.
//!
//! WHAT: the compiler-owned stage sequence for one authored `config.moth` — tokenization,
//!       declaration-shell preparation, interface binding for the single authored source, local
//!       declaration ordering, AST folding and folded-value projection — stopping at owned
//!       folded declarations.
//! WHY:  config is written in normal Moth syntax but must bootstrap before source-package discovery
//!       exists, so it needs a shorter path than canonical module compilation. That path is a named
//!       compiler service rather than a build-owned stage sequence: the build system supplies the
//!       source and consumes folded declaration values, and never composes preparation, binding,
//!       ordering or AST itself.
//!
//! The service produces no HIR, borrow facts, link facts or public interface. It owns the config
//! dialect surface — which declaration shapes, start-body statements and dependency clauses
//! `config.moth` accepts — and the authored key-name spans config diagnostics underline. Config
//! key schema and the application of folded values to project settings stay build-owned.

use crate::builder_surface::config_schema::ProjectFieldConfigPolicies;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::{Ast, AstBuildContext, AstBuildInput};
use crate::compiler_frontend::build_config::{
    BuildConfigInputSet, BuilderConfigGlobalSet, ConfigResolutionRecord, ConfigResolutionServices,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeIdentity, CanonicalTypeProjectionContext, NominalOriginResolver,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticBag, DiagnosticKind,
    InvalidConfigReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::declaration_syntax::build_config_contract::find_invalid_config_qualifier_spacing;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, FoldedValueProjectionContext, PublicFoldedValue,
    convert_const_value_to_folded_value,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure, FileFrontendPrepareOutput, Header,
    HeaderKind, HeaderParseOptions, HeaderPreparationFailure, bind_module_headers,
    prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, OriginTypeId};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceDatabase, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathInternError, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::{TokenizeFailure, tokenize};
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

use std::collections::HashMap;
use std::path::Path;

/// One authored config source and the capability surface it compiles against.
pub(crate) struct ConfigCompilationRequest<'a> {
    /// The config path exactly as the project spelled it.
    ///
    /// WHY: this is the authored identity every config diagnostic reports and every authored-scope
    ///      comparison uses, so it must not be replaced by the canonical form.
    pub(crate) authored_path: &'a Path,
    /// The canonical filesystem path the authored config resolved to.
    pub(crate) canonical_path: &'a Path,
    /// The registered authored source identity carried by this config's token stream.
    pub(crate) file_id: SourceId,
    pub(crate) source_code: &'a str,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) binding_packages: &'a ExternalPackageRegistry,
    /// Typed explicit command/programmatic inputs for direct project qualifiers.
    pub(crate) build_config_inputs: &'a BuildConfigInputSet,
    /// Typed platform-neutral primitive globals supplied by the selected builder.
    pub(crate) builder_config_globals: &'a BuilderConfigGlobalSet,
    /// Builder-schema policy for direct grouped-project fields.
    pub(crate) project_field_config_policies: ProjectFieldConfigPolicies,
}
/// The folded config source a caller validates and applies.
pub(crate) struct CompiledConfigSource {
    /// One owned folded declaration per authored top-level compile-time constant, in the
    /// declaration-table order the module store produces.
    pub(crate) declarations: Vec<FoldedConfigDeclaration>,
    /// Direct-project qualifier resolution facts retained for later compiler phases.
    #[allow(dead_code)]
    pub(crate) resolution_records: Vec<ConfigResolutionRecord>,
}

/// The semantic config result plus the source-local span owner produced while compiling it.
///
/// The build caller retains the original builder through config validation and installs its
/// frozen table only after the last producer, including diagnosed and infrastructure outcomes.
pub(crate) struct ConfigCompilationOutcome {
    pub(crate) result: Result<CompiledConfigSource, CompilerMessages>,
    pub(crate) file_id: SourceId,
    pub(crate) span_builder: ExtendedSpanBuilder,
}

/// WHAT: carries the authored key name, its owned folded value, and exact optional spans for the
///       declaration and key name. Direct record-field initializer spans remain aligned with the
///       folded fields. Public folded values stay location-free; provenance is retained only in
///       these exact span fields.
/// WHY: build-side validation consumes owned values with no donor-local AST identity, so the
///       compiler service resolves every donor-local handle while the module is still in scope.
pub(crate) struct FoldedConfigDeclaration {
    pub(crate) name: StringId,
    pub(crate) value: PublicFoldedValue,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) name_span: Option<SourceSpan>,
    pub(crate) direct_field_spans: Vec<Option<SourceSpan>>,
}

/// Compile one authored `config.moth` to owned folded declarations and retain its source spans.
///
/// The service returns its live source builder on every outcome; the build caller owns finalization.
pub(crate) fn compile_config_source(
    request: ConfigCompilationRequest<'_>,
    string_table: &mut StringTable,
) -> ConfigCompilationOutcome {
    let file_id = request.file_id;
    let mut path_fork = PathInternerFork::empty();

    // Construct the authored logical identity in the config service's path domain before
    // tokenization and header preparation. The service is self-contained, so its fork owns the
    // complete config path table.
    let authored_scope = match path_fork.try_intern_filesystem_path(request.authored_path, string_table)
    {
        Ok(scope) => scope,
        Err(PathInternError::NonUtf8(non_utf8)) => {
            return ConfigCompilationOutcome {
                result: Err(CompilerMessages::from_error(
                    CompilerError::file_error(
                        &non_utf8.path,
                        format!(
                            "Config path {:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths.",
                            non_utf8.path
                        ),
                    ),
                    string_table.clone(),
                )),
                file_id,
                span_builder: ExtendedSpanBuilder::new(),
            };
        }
        Err(PathInternError::TableFull) => {
            return ConfigCompilationOutcome {
                result: Err(CompilerMessages::from_error(
                    CompilerError::compiler_error(
                        "Config path table exhausted while interning the authored path",
                    ),
                    string_table.clone(),
                )),
                file_id,
                span_builder: ExtendedSpanBuilder::new(),
            };
        }
    };

    let mut span_builder = ExtendedSpanBuilder::new();
    match prepare_config_file(
        &request,
        authored_scope,
        string_table,
        &mut path_fork,
        &mut span_builder,
    ) {
        Ok(mut file) => {
            let result = compile_prepared_config_source(
                &request,
                authored_scope,
                &mut file,
                string_table,
                &mut path_fork,
            );
            ConfigCompilationOutcome {
                result,
                file_id,
                span_builder,
            }
        }
        Err(ConfigPreparationFailure::Diagnosed(diagnostics)) => ConfigCompilationOutcome {
            result: Err(CompilerMessages::from_diagnostics(
                diagnostics,
                string_table.clone(),
            )),
            file_id,
            span_builder,
        },
        Err(ConfigPreparationFailure::Infrastructure(error)) => ConfigCompilationOutcome {
            result: Err(CompilerMessages::from_error_ref(error, string_table)),
            file_id,
            span_builder,
        },
    }
}

/// Run config-specific aggregation, binding, ordering, AST folding and folded-value projection.
///
/// Header aggregation consumes the retained shells while the outer service owns their span builder.
fn compile_prepared_config_source(
    request: &ConfigCompilationRequest<'_>,
    authored_scope: PathId,
    prepared_file: &mut FileFrontendPrepareOutput,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<CompiledConfigSource, CompilerMessages> {
    let prepared = prepare_header_syntax(
        std::slice::from_mut(prepared_file),
        string_table,
        &mut |source, diagnostic| {
            if source != request.file_id {
                return Err(CompilerError::compiler_error(
                    "config aggregation used a different source owner".to_owned(),
                ));
            }
            diagnostic.capture_preparation_span(source)
        },
        path_fork,
    )
    .map_err(|failure| match failure {
        HeaderPreparationFailure::Diagnosed(bag) => CompilerMessages::from_diagnostics(
            classify_header_diagnostics(bag, request.file_id),
            string_table.clone(),
        ),
        HeaderPreparationFailure::Infrastructure(error) => {
            CompilerMessages::from_error_ref(error, string_table)
        }
    })?;
    let bound_headers = bind_module_headers(
        prepared,
        request.binding_packages,
        &ExternalImportResolutionTable::default(),
        &SourceProviderDependencySet::default(),
        None,
        &SourceDatabase::empty(),
        string_table,
        path_fork,
    )
    .map_err(|failure| match failure {
        HeaderPreparationFailure::Diagnosed(bag) => CompilerMessages::from_diagnostics(
            classify_header_diagnostics(bag, request.file_id),
            string_table.clone(),
        ),
        HeaderPreparationFailure::Infrastructure(error) => {
            CompilerMessages::from_error_ref(error, string_table)
        }
    })?;

    // Order local declarations.
    let sorted =
        resolve_module_dependencies(
            bound_headers,
            &ContentSourceTargets::empty(),
            string_table,
            path_fork,
        )
        .map_err(|failure| failure.into_messages(string_table))?;

    // Preserve key-name spans before AST consumes the headers. The full header path becomes the
    // declaration ID, so every folded declaration can carry its exact authored name span.
    let authored_key_name_provenance =
        collect_authored_config_key_name_provenance(&sorted.headers, authored_scope);

    // Fold the ordered declarations. Config stops here: no HIR, borrow facts or interface.
    let config_resolution = ConfigResolutionServices::new(
        request.build_config_inputs,
        request.builder_config_globals,
        request.project_field_config_policies.clone(),
    );
    let ast = Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            module_symbols: sorted.module_symbols,
            binding_environment: sorted.binding_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
            source_build_config_contract_names: std::sync::Arc::new(Default::default()),
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry: std::sync::Arc::new(request.binding_packages.clone()),
            style_directives: request.style_directives,
            string_table,
            path_fork,
            entry_dir: authored_scope,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution: None,
            config_resolution: Some(std::rc::Rc::clone(&config_resolution)),
            build_config_values: std::sync::Arc::new(Default::default()),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Config,
        },
    )?
    .ast;

    // Reject authored start-body statements and mutable config bindings. Only top-level
    // compile-time constants are config entries, so these dialect rejections are owned by
    // this service and never reach build-side validation.
    let config_rejections =
        reject_authored_config_dialect(&ast, authored_scope, path_fork, string_table);
    if !config_rejections.is_empty() {
        return Err(CompilerMessages::from_diagnostics(
            config_rejections,
            string_table.clone(),
        ));
    }

    // Project every authored top-level folded constant into the owned folded-value vocabulary
    // while the donor-local type environment and string table are still in scope.
    let declarations = project_authored_config_declarations(
        &ast,
        authored_scope,
        &authored_key_name_provenance,
        request.binding_packages,
        string_table,
        path_fork,
    )?;

    Ok(CompiledConfigSource {
        declarations,
        resolution_records: config_resolution.take_records(),
    })
}

// -------------------------
//  Config Dialect Rejections After Folding
// -------------------------

/// Reject the authored config dialect surfaces that fold but are not config entries.
///
/// Only top-level compile-time constants are config entries. Plain bindings and runtime
/// statements in the start body, and mutable config bindings, are rejected here so build-side
/// validation consumes folded declarations without walking AST nodes or inspecting value modes.
fn reject_authored_config_dialect(
    ast: &Ast,
    authored_scope: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut rejections =
        reject_authored_config_start_body(ast, authored_scope, path_fork, string_table);
    rejections.extend(reject_mutable_config_bindings(ast, authored_scope, path_fork));
    rejections
}

/// Reject authored statements inside the config's start body.
///
/// The implicit or authored `start` body is the only place runtime statements can reach the
/// folded AST, so the walk is scoped to the start function and to statements authored in the
/// config file itself.
fn reject_authored_config_start_body(
    ast: &Ast,
    authored_scope: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut rejections = Vec::new();

    for node in &ast.nodes {
        let NodeKind::Function(path, _, body) = &node.kind else {
            continue;
        };

        if path_fork.component(*path).map(|name| string_table.resolve(name))
            != Some(IMPLICIT_START_FUNC_NAME)
        {
            continue;
        }

        for body_node in body {
            // Only consider statements authored in the config file itself.
            if body_node.scope != authored_scope {
                continue;
            }

            match &body_node.kind {
                NodeKind::VariableDeclaration(declaration) => {
                    let key = path_fork.component(declaration.id);
                    rejections.push(config_diagnostic(
                        key,
                        InvalidConfigReason::PlainBindingUnsupported,
                        declaration.value.span,
                    ));
                }
                NodeKind::PushStartRuntimeFragment(_) => rejections.push(config_diagnostic(
                    None,
                    InvalidConfigReason::StandaloneTemplateUnsupported,
                    body_node.span,
                )),
                _ => rejections.push(config_diagnostic(
                    None,
                    InvalidConfigReason::UnsupportedStatement,
                    body_node.span,
                )),
            }
        }
    }

    rejections
}

/// Reject mutable top-level bindings that reached the folded module store.
fn reject_mutable_config_bindings(
    ast: &Ast,
    authored_scope: PathId,
    path_fork: &PathInternerFork,
) -> Vec<CompilerDiagnostic> {
    let mut rejections = Vec::new();

    for row in ast.const_values.iter_module_constant_views() {
        let (path, metadata) = (row.path, row.metadata);
        if path_fork.try_parent(*path) != Some(authored_scope) {
            continue;
        }

        if metadata.value_mode.is_mutable() {
            rejections.push(config_diagnostic(
                path_fork.component(*path),
                InvalidConfigReason::MutableBindingUnsupported,
                metadata.span,
            ));
        }
    }

    rejections
}

// -------------------------
//  Folded Declaration Projection
// -------------------------

/// WHAT: iterates the module store's declaration-table rows, keeps the authored-scope constants
///       and converts each one through the shared folded-value converter, resolving donor-local
///       string and type identities while the module is still in scope. Record values also
///       project initializer spans in the same order as the folded fields.
/// WHY:  the owned folded-value vocabulary is the one boundary shape build-side validation
///       consumes; no donor-local AST, const-store or type identity may cross it.
fn project_authored_config_declarations(
    ast: &Ast,
    authored_scope: PathId,
    authored_key_name_provenance: &HashMap<PathId, Option<SourceSpan>>,
    binding_packages: &ExternalPackageRegistry,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Result<Vec<FoldedConfigDeclaration>, CompilerMessages> {
    let nominal_origins = ConfigNominalOriginResolver {
        type_environment: &ast.type_environment,
    };
    let generic_parameter_origins = FoldedValueGenericParameterResolver;
    let projection_context = CanonicalTypeProjectionContext::new(
        &nominal_origins,
        &generic_parameter_origins,
        binding_packages,
    );
    let folded_value_context = FoldedValueProjectionContext {
        type_environment: &ast.type_environment,
        string_table,
        projection_context: &projection_context,
        resources: None,
        path_fork,
    };

    let mut declarations = Vec::new();
    for row in ast.const_values.iter_module_constant_views() {
        let (path, value_id, metadata) = (row.path, row.id, row.metadata);

        // A module constant's source file is the parent of its symbol path, so the authored
        // scope is checked directly in the module-local path table.
        if path_fork.try_parent(*path) != Some(authored_scope) {
            continue;
        }

        let Some(name) = path_fork.component(*path) else {
            continue;
        };

        let value =
            convert_const_value_to_folded_value(&ast.const_values, value_id, &folded_value_context)
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
        let direct_field_spans = project_direct_field_spans(&ast.const_values, value_id)
            .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
        if let PublicFoldedValue::Record(fields) = &value {
            if fields.len() != direct_field_spans.len() {
                return Err(CompilerMessages::from_error(
                    CompilerError::compiler_error(
                        "config field provenance must align with folded record fields",
                    ),
                    string_table.clone(),
                ));
            }
        } else if !direct_field_spans.is_empty() {
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(
                    "config field provenance was projected for a non-record value",
                ),
                string_table.clone(),
            ));
        }

        let name_span = authored_key_name_provenance
            .get(path)
            .copied()
            .flatten()
            .or(metadata.span);

        declarations.push(FoldedConfigDeclaration {
            name,
            value,
            span: metadata.span,
            name_span,
            direct_field_spans,
        });
    }

    Ok(declarations)
}

/// Project direct record-field initializer spans in folded-field order.
fn project_direct_field_spans(
    const_values: &ConstValueStore,
    value_id: ConstValueId,
) -> Result<Vec<Option<SourceSpan>>, CompilerError> {
    let Some(payload) = const_values.payload(value_id) else {
        return Err(CompilerError::compiler_error(
            "config field-span projection: missing const-store value",
        ));
    };

    match payload {
        ConstValuePayload::Record(fields) => Ok(fields
            .iter()
            .map(|field| {
                const_values
                    .metadata(field.value)
                    .and_then(|metadata| metadata.span)
            })
            .collect()),

        ConstValuePayload::OptionSome(inner) | ConstValuePayload::Coerced(inner) => {
            project_direct_field_spans(const_values, *inner)
        }

        _ => Ok(Vec::new()),
    }
}

/// Resolves config folded-value nominal origins through the module's registered canonical
/// identities.
///
/// WHAT: reads the canonical identity the config module's type environment registered for a
///       nominal. Config rejects dependency clauses, so it registers no imported nominal and
///       declares no exported origin.
/// WHY:  the shared folded-value projection needs one `NominalOriginResolver`; a config nominal
///       without a registered canonical identity has no owned boundary identity to project.
struct ConfigNominalOriginResolver<'a> {
    type_environment: &'a TypeEnvironment,
}

impl NominalOriginResolver for ConfigNominalOriginResolver<'_> {
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError> {
        let type_id = self
            .type_environment
            .type_id_for_nominal_id(nominal_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Config folded-value projection has an unknown nominal type",
                )
            })?;
        match self
            .type_environment
            .canonical_identity_for_type_id(type_id)
        {
            Some(CanonicalTypeIdentity::SourceNominal(origin)) => Ok(origin.clone()),
            _ => Err(CompilerError::compiler_error(
                "Config folded-value projection has no source nominal origin",
            )),
        }
    }
}

enum ConfigPreparationFailure {
    Diagnosed(Vec<CompilerDiagnostic>),
    Infrastructure(CompilerError),
}

// -------------------------
//  Per-File Preparation
// -------------------------

/// Tokenize and header-parse the single authored config file, then apply the config dialect surface.
///
/// Dependency clauses are rejected from the retained structural shell before interface binding can
/// resolve a package or filesystem target. Every span tokenization encodes lands in the caller's
/// builder, which survives success, diagnosed rejection and infrastructure failure alike.
fn prepare_config_file(
    request: &ConfigCompilationRequest<'_>,
    authored_scope: PathId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, ConfigPreparationFailure> {
    let mut diagnostics = Vec::new();

    // The caller registers this source before invoking the service.
    let mut file_tokens = match tokenize(
        request.source_code,
        authored_scope,
        TokenizerEntryMode::SourceFile,
        request.style_directives,
        string_table,
        path_fork,
        request.file_id,
        span_builder,
    ) {
        Ok(output) => output,
        Err(TokenizeFailure::Diagnosed(diagnostic)) => {
            diagnostics.push(diagnostic);
            return Err(ConfigPreparationFailure::Diagnosed(diagnostics));
        }
        Err(TokenizeFailure::Infrastructure(error)) => {
            return Err(ConfigPreparationFailure::Infrastructure(error));
        }
    };
    file_tokens.canonical_os_path = Some(request.canonical_path.to_path_buf());

    if let Some(marker_span) = find_invalid_config_qualifier_spacing(
        &file_tokens.tokens,
        string_table,
        request.file_id,
        span_builder,
    ) {
        let mut diagnostic = CompilerDiagnostic::common_syntax_mistake(
            CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
            Some(marker_span),
        );
        diagnostic
            .capture_preparation_span(request.file_id)
            .map_err(ConfigPreparationFailure::Infrastructure)?;
        diagnostics.push(diagnostic);
        return Err(ConfigPreparationFailure::Diagnosed(diagnostics));
    }

    let output = match prepare_file_from_tokens(
        file_tokens,
        request.authored_path,
        &HeaderParseOptions::default(),
        string_table,
        0,
        0,
        span_builder,
        path_fork,
    ) {
        Ok(output) => output,
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
            let FileFrontendPrepareError {
                warnings,
                diagnostic,
                ..
            } = error;
            diagnostics.extend(warnings);
            if is_duplicate_config_header_error(&diagnostic) {
                diagnostics.push(config_duplicate_diagnostic(diagnostic));
            } else {
                diagnostics.push(diagnostic);
            }
            return Err(ConfigPreparationFailure::Diagnosed(diagnostics));
        }
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            return Err(ConfigPreparationFailure::Infrastructure(error));
        }
    };

    for dependency_clause in &output.file_dependency_clauses {
        diagnostics.push(config_diagnostic(
            None,
            InvalidConfigReason::ConfigImportUnsupported,
            Some(dependency_clause.dependency.span),
        ));
    }
    for file_reference in output.structural_file_references.iter() {
        diagnostics.push(config_diagnostic(
            None,
            InvalidConfigReason::FileValuePathUnsupported,
            Some(file_reference.span),
        ));
    }
    diagnostics.extend(validate_authored_config_surface(
        &output.headers,
        path_fork,
        string_table,
    ));
    for diagnostic in &mut diagnostics {
        diagnostic
            .capture_preparation_span(request.file_id)
            .map_err(ConfigPreparationFailure::Infrastructure)?;
    }

    if diagnostics.is_empty() {
        Ok(output)
    } else {
        Err(ConfigPreparationFailure::Diagnosed(diagnostics))
    }
}

// -------------------------
//  Authored Key-Name Spans
// -------------------------

/// Collect authored key-name spans for config key-identity diagnostics.
///
/// Imported support declarations are excluded because they are not config entries.
fn collect_authored_config_key_name_provenance(
    headers: &[Header],
    authored_scope: PathId,
) -> HashMap<PathId, Option<SourceSpan>> {
    let mut key_name_provenance = HashMap::new();
    for header in headers {
        let HeaderKind::Constant { .. } = &header.kind else {
            continue;
        };
        if header.source_file != authored_scope {
            continue;
        }
        key_name_provenance.insert(header.tokens.src_path, header.name_span);
    }
    key_name_provenance
}

// -------------------------
//  Config Dialect Surface
// -------------------------

/// Reject unsupported surfaces in the authored `config.moth` file after header parsing has
/// normalized declaration shapes.
///
/// WHY: config uses frontend parsing for expression semantics, but config is not a normal module.
/// It is compile-time-only, so runtime declarations such as functions and standalone templates are
/// rejected before AST. Named support types — structs, choices and type aliases — are equally
/// rejected: user-authored `config.moth` is a flat surface of anonymous const records and
/// compiler-owned constants, and record-shaped helpers must be declared as anonymous const
/// records and referenced by name. Trait surfaces are source-module metadata and are
/// deliberately kept out of config.
/// Dependency clauses are rejected from `FileFrontendPrepareOutput.file_dependency_clauses` before
/// this declaration validation. Authored start-body statements are rejected after AST folding, in
/// this service.
fn validate_authored_config_surface(
    headers: &[Header],
    path_fork: &PathInternerFork,
    _string_table: &StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut errors = Vec::new();

    for header in headers {
        let reason = match &header.kind {
            HeaderKind::Function { .. } => Some(InvalidConfigReason::FunctionUnsupported),
            HeaderKind::ConstTemplate { .. } => {
                Some(InvalidConfigReason::StandaloneTemplateUnsupported)
            }
            HeaderKind::Trait { .. } => Some(InvalidConfigReason::TraitDeclarationUnsupported),
            HeaderKind::TraitConformance { .. } => {
                Some(InvalidConfigReason::TraitConformanceUnsupported)
            }
            HeaderKind::TraitIncompatibility { .. } => {
                Some(InvalidConfigReason::TraitIncompatibilityUnsupported)
            }
            HeaderKind::Constant { .. } | HeaderKind::StartFunction => None,
            HeaderKind::Struct { .. }
            | HeaderKind::Choice { .. }
            | HeaderKind::TypeAlias { .. } => Some(InvalidConfigReason::NamedTypeUnsupported),
        };

        if let Some(reason) = reason {
            errors.push(config_diagnostic(
                path_fork.component(header.tokens.src_path),
                reason,
                header.name_span,
            ));
        }
    }

    errors
}

// -------------------------
//  Duplicate Classification
// -------------------------

/// Re-route an authored duplicate declaration to the config key vocabulary.
fn classify_header_diagnostics(
    bag: DiagnosticBag,
    authored_file_id: SourceId,
) -> Vec<CompilerDiagnostic> {
    bag.into_diagnostics()
        .into_iter()
        .map(|diagnostic| {
            if is_authored_config_duplicate(&diagnostic, authored_file_id) {
                config_duplicate_diagnostic(diagnostic)
            } else {
                diagnostic
            }
        })
        .collect()
}

/// Config displays one key label for duplicates, retaining its captured authored span.
fn config_duplicate_diagnostic(diagnostic: CompilerDiagnostic) -> CompilerDiagnostic {
    config_diagnostic(
        None,
        InvalidConfigReason::DuplicateKey,
        diagnostic.primary_span,
    )
}

fn is_duplicate_config_header_error(diagnostic: &CompilerDiagnostic) -> bool {
    matches!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    )
}

fn is_authored_config_duplicate(
    diagnostic: &CompilerDiagnostic,
    authored_file_id: SourceId,
) -> bool {
    is_duplicate_config_header_error(diagnostic)
        && diagnostic
            .primary_span
            .is_some_and(|span| span.source() == authored_file_id)
}

fn config_diagnostic(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(key, reason, span)
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;

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
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, DiagnosticKind, InvalidConfigReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, FoldedValueProjectionContext, PublicFoldedValue,
    convert_const_value_to_folded_value,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, Header, HeaderKind, HeaderParseOptions,
    bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, OriginTypeId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
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

/// One authored top-level config constant projected to the owned folded-value vocabulary.
///
/// WHAT: carries the authored key name, its owned folded value, the two spans config
///       diagnostics underline, and a location-only table aligned with direct record fields.
///       Public folded values stay location-free; each `direct_field_locations[i]` is the
///       initializer span for `Record` field `i`.
/// WHY:  build-side validation consumes owned values with no donor-local AST identity, so the
///       compiler service resolves every donor-local handle while the module is still in scope.
pub(crate) struct FoldedConfigDeclaration {
    pub(crate) name: StringId,
    pub(crate) value: PublicFoldedValue,
    pub(crate) location: SourceLocation,
    pub(crate) name_location: SourceLocation,
    pub(crate) direct_field_locations: Vec<SourceLocation>,
}

/// Compile one authored `config.moth` to owned folded declarations.
///
/// Every stage failure is returned as diagnostics; nothing partial is handed back.
pub(crate) fn compile_config_source(
    request: ConfigCompilationRequest<'_>,
    string_table: &mut StringTable,
) -> Result<CompiledConfigSource, CompilerMessages> {
    // Construct the one exact authored `InternedPath` before file preparation and reuse it for
    // tokenization, AST entry identity and diagnostic ownership.
    let authored_scope = InternedPath::try_from_filesystem_path(request.authored_path, string_table)
        .map_err(|non_utf8| {
            CompilerMessages::from_error(
                CompilerError::file_error(
                    &non_utf8.path,
                    format!(
                        "Config path {:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths.",
                        non_utf8.path
                    ),
                    string_table,
                ),
                string_table.clone(),
            )
        })?;

    // 1. Tokenize and prepare the single authored file, then apply the config dialect surface.
    let mut surface_errors = Vec::new();
    let prepared_file =
        prepare_config_file(&request, &authored_scope, &mut surface_errors, string_table)?;
    if !surface_errors.is_empty() {
        return Err(CompilerMessages::from_diagnostics(
            surface_errors,
            string_table.clone(),
        ));
    }
    let prepared_file = prepared_file.into_iter().collect::<Vec<_>>();

    // 2. Aggregate retained syntax and bind it against the builder's provider interfaces.
    //
    // WHY: syntax preparation is provider-independent and binding resolves retained shells against
    //      provider interfaces. Both phases share the same duplicate-key diagnostic routing, so the
    //      error path is classified once.
    let bound_headers =
        match prepare_header_syntax(prepared_file, string_table).and_then(|prepared| {
            bind_module_headers(
                prepared,
                request.binding_packages,
                &ExternalImportResolutionTable::default(),
                &SourceProviderDependencySet::default(),
                None,
                string_table,
            )
        }) {
            Ok(headers) => headers,
            Err(bag) => {
                return Err(CompilerMessages::from_diagnostics(
                    classify_header_diagnostics(bag, &authored_scope),
                    string_table.clone(),
                ));
            }
        };

    // 3. Order local declarations.
    let sorted =
        resolve_module_dependencies(bound_headers, &ContentSourceTargets::empty(), string_table)
            .map_err(|bag| {
                CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone())
            })?;

    // 4. Preserve key-name spans before AST consumes the headers. The full header path becomes the
    //    declaration ID, so every folded declaration can carry its exact authored name span.
    let authored_key_name_locations =
        collect_authored_config_key_name_locations(&sorted.headers, &authored_scope);

    // 5. Fold the ordered declarations. Config stops here: no HIR, borrow facts or interface.
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
            entry_dir: authored_scope.clone(),
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

    // 6. Reject authored start-body statements and mutable config bindings. Only top-level
    //    compile-time constants are config entries, so these dialect rejections are owned by
    //    this service and never reach build-side validation.
    let config_rejections = reject_authored_config_dialect(&ast, &authored_scope, string_table);
    if !config_rejections.is_empty() {
        return Err(CompilerMessages::from_diagnostics(
            config_rejections,
            string_table.clone(),
        ));
    }

    // 7. Project every authored top-level folded constant into the owned folded-value vocabulary
    //    while the donor-local type environment and string table are still in scope. Config
    //    rejects file-value paths, so the projection needs no module resource table.
    let declarations = project_authored_config_declarations(
        &ast,
        &authored_scope,
        &authored_key_name_locations,
        request.binding_packages,
        string_table,
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
    authored_scope: &InternedPath,
    string_table: &mut StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut rejections = reject_authored_config_start_body(ast, authored_scope, string_table);
    rejections.extend(reject_mutable_config_bindings(ast, authored_scope));
    rejections
}

/// Reject authored statements inside the config's start body.
///
/// The implicit or authored `start` body is the only place runtime statements can reach the
/// folded AST, so the walk is scoped to the start function and to statements authored in the
/// config file itself.
fn reject_authored_config_start_body(
    ast: &Ast,
    authored_scope: &InternedPath,
    string_table: &mut StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut rejections = Vec::new();

    for node in &ast.nodes {
        let NodeKind::Function(path, _, body) = &node.kind else {
            continue;
        };

        if path.name_str(string_table) != Some(IMPLICIT_START_FUNC_NAME) {
            continue;
        }

        for body_node in body {
            // Only consider statements authored in the config file itself.
            if body_node.scope != *authored_scope {
                continue;
            }

            match &body_node.kind {
                NodeKind::VariableDeclaration(declaration) => {
                    let key = declaration
                        .id
                        .name_str(string_table)
                        .unwrap_or("")
                        .to_string();
                    rejections.push(config_diagnostic(
                        Some(string_table.intern(&key)),
                        InvalidConfigReason::PlainBindingUnsupported,
                        declaration.value.location.clone(),
                    ));
                }
                NodeKind::PushStartRuntimeFragment(_) => rejections.push(config_diagnostic(
                    None,
                    InvalidConfigReason::StandaloneTemplateUnsupported,
                    body_node.location.clone(),
                )),
                _ => rejections.push(config_diagnostic(
                    None,
                    InvalidConfigReason::UnsupportedStatement,
                    body_node.location.clone(),
                )),
            }
        }
    }

    rejections
}

/// Reject mutable top-level bindings that reached the folded module store.
fn reject_mutable_config_bindings(
    ast: &Ast,
    authored_scope: &InternedPath,
) -> Vec<CompilerDiagnostic> {
    let mut rejections = Vec::new();

    for row in ast.const_values.iter_module_constant_views() {
        let (path, metadata) = (row.path, row.metadata);
        if path.parent().as_ref() != Some(authored_scope) {
            continue;
        }

        if metadata.value_mode.is_mutable() {
            rejections.push(config_diagnostic(
                path.name(),
                InvalidConfigReason::MutableBindingUnsupported,
                metadata.location.clone(),
            ));
        }
    }

    rejections
}

// -------------------------
//  Folded Declaration Projection
// -------------------------

/// Project every authored top-level folded constant to the owned folded-value vocabulary.
///
/// WHAT: iterates the module store's declaration-table rows, keeps the authored-scope constants
///       and converts each one through the shared folded-value converter, resolving donor-local
///       string and type identities while the module is still in scope. Record values also
///       project initializer locations in the same order as the folded fields.
/// WHY:  the owned folded-value vocabulary is the one boundary shape build-side validation
///       consumes; no donor-local AST, const-store or type identity may cross it.
fn project_authored_config_declarations(
    ast: &Ast,
    authored_scope: &InternedPath,
    authored_key_name_locations: &HashMap<InternedPath, SourceLocation>,
    binding_packages: &ExternalPackageRegistry,
    string_table: &StringTable,
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
    };

    let mut declarations = Vec::new();
    for row in ast.const_values.iter_module_constant_views() {
        let (path, value_id, metadata) = (row.path, row.id, row.metadata);

        // A module constant's source file is the parent of its symbol path, so the authored
        // scope is checked by direct interned equality rather than by converting paths back to
        // `PathBuf`.
        if path.parent().as_ref() != Some(authored_scope) {
            continue;
        }

        let Some(name) = path.name() else {
            continue;
        };

        let value =
            convert_const_value_to_folded_value(&ast.const_values, value_id, &folded_value_context)
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
        let direct_field_locations = project_direct_field_locations(&ast.const_values, value_id)
            .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
        if let PublicFoldedValue::Record(fields) = &value {
            if fields.len() != direct_field_locations.len() {
                return Err(CompilerMessages::from_error(
                    CompilerError::compiler_error(
                        "config field locations must align with folded record fields",
                    ),
                    string_table.clone(),
                ));
            }
        } else if !direct_field_locations.is_empty() {
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(
                    "config field locations were projected for a non-record value",
                ),
                string_table.clone(),
            ));
        }

        declarations.push(FoldedConfigDeclaration {
            name,
            value,
            location: metadata.location.clone(),
            name_location: authored_key_name_locations
                .get(path)
                .cloned()
                .unwrap_or_else(|| metadata.location.clone()),
            direct_field_locations,
        });
    }

    Ok(declarations)
}

/// Project direct record-field initializer locations in folded-field order.
fn project_direct_field_locations(
    const_values: &ConstValueStore,
    value_id: ConstValueId,
) -> Result<Vec<SourceLocation>, CompilerError> {
    let Some(payload) = const_values.payload(value_id) else {
        return Err(CompilerError::compiler_error(
            "config field-location projection: missing const-store value",
        ));
    };

    match payload {
        ConstValuePayload::Record(fields) => {
            Ok(fields.iter().map(|field| field.location.clone()).collect())
        }

        ConstValuePayload::OptionSome(inner) | ConstValuePayload::Coerced(inner) => {
            project_direct_field_locations(const_values, *inner)
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

// -------------------------
//  Per-File Preparation
// -------------------------

/// Tokenize and header-parse the single authored config file, then apply the config dialect surface.
///
/// Dependency clauses are rejected from the retained structural shell before interface binding can
/// resolve a package or filesystem target.
fn prepare_config_file(
    request: &ConfigCompilationRequest<'_>,
    authored_scope: &InternedPath,
    errors: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<Option<FileFrontendPrepareOutput>, CompilerMessages> {
    // The authored scope identity is already interned, so tokenization reuses it directly without a
    // second `InternedPath::try_from_filesystem_path` round-trip.
    // Config is one self-contained file and never participates in provider binding, so the
    // placeholder file identity only stamps shells that are rejected as `ConfigImportUnsupported`
    // immediately after preparation. It is intentionally isolated from every module/package
    // identity space.
    let mut token_stream = match tokenize(
        request.source_code,
        authored_scope,
        TokenizerEntryMode::SourceFile,
        request.style_directives,
        string_table,
        Some(FileId(0)),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            errors.push(*error);
            return Ok(None);
        }
    };
    token_stream.canonical_os_path = Some(request.canonical_path.to_path_buf());

    let output = match prepare_file_from_tokens(
        token_stream,
        request.authored_path,
        &HeaderParseOptions::default(),
        string_table,
        0,
        0,
    ) {
        Ok(output) => output,
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
            errors.extend(error.warnings);
            if is_duplicate_config_header_error(&error.diagnostic) {
                errors.push(config_diagnostic(
                    None,
                    InvalidConfigReason::DuplicateKey,
                    error.diagnostic.primary_location.clone(),
                ));
            } else {
                errors.push(*error.diagnostic);
            }
            return Ok(None);
        }
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };

    for dependency_clause in &output.file_dependency_clauses {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::ConfigImportUnsupported,
            dependency_clause.location.clone(),
        ));
    }
    for file_reference in output.structural_file_references.iter() {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::FileValuePathUnsupported,
            file_reference.location.clone(),
        ));
    }
    errors.extend(validate_authored_config_surface(&output.headers));

    Ok(Some(output))
}

// -------------------------
//  Authored Key-Name Spans
// -------------------------

/// Collect the authored key-name spans for config key-identity diagnostics.
///
/// Imported support declarations are excluded because they are not config entries.
fn collect_authored_config_key_name_locations(
    headers: &[Header],
    authored_scope: &InternedPath,
) -> HashMap<InternedPath, SourceLocation> {
    let mut key_name_locations = HashMap::new();
    for header in headers {
        let HeaderKind::Constant { .. } = &header.kind else {
            continue;
        };
        if header.source_file != *authored_scope {
            continue;
        }
        key_name_locations.insert(
            header.tokens.src_path.to_owned(),
            header.name_location.clone(),
        );
    }
    key_name_locations
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
fn validate_authored_config_surface(headers: &[Header]) -> Vec<CompilerDiagnostic> {
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
                header.tokens.src_path.name(),
                reason,
                header.name_location.clone(),
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
    authored_scope: &InternedPath,
) -> Vec<CompilerDiagnostic> {
    bag.into_diagnostics()
        .into_iter()
        .map(|diagnostic| {
            if is_authored_config_duplicate(&diagnostic, authored_scope) {
                config_diagnostic(
                    None,
                    InvalidConfigReason::DuplicateKey,
                    diagnostic.primary_location.clone(),
                )
            } else {
                diagnostic
            }
        })
        .collect()
}

fn is_duplicate_config_header_error(diagnostic: &CompilerDiagnostic) -> bool {
    matches!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    )
}

fn is_authored_config_duplicate(
    diagnostic: &CompilerDiagnostic,
    authored_scope: &InternedPath,
) -> bool {
    // Classify authored duplicate declarations by direct interned scope equality.
    // WHY: the authored config file was tokenized with this exact interned identity, so a
    // duplicate declaration whose primary location shares that scope is an authored duplicate.
    // Comparing interned identity avoids converting paths back to `PathBuf` or canonicalizing
    // during diagnostic handling.
    is_duplicate_config_header_error(diagnostic)
        && diagnostic.primary_location.scope == *authored_scope
}

fn config_diagnostic(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(key, reason, location)
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;

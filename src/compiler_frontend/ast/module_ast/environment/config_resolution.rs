//! AST declaration-owned configuration resolution.
//!
//! WHAT: resolves declaration-owned `#Config of T` metadata and seeds one stable
//!       `SyntheticInterfaceProvenance` input fact per declaration-owned input.
//! WHY: configuration validation and primitive materialisation are a focused semantic phase,
//!      separate from the ordinary top-level constant session and its compile-time fold.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, OccurrenceFacts, OwnedExpr,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template_folding::TemplateEmission;
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TirView,
    fold_prepared_template, prepare_tir_view,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::build_config::{
    BuildConfigValueLocation, BuildConfigValueOrigin, BuildInputName, BuildInputType,
    ConfigResolutionRecord, ConfigResolutionServices, ContractValueError, PrimitiveBuildInputType,
    PrimitiveBuildValue, ResolvedBuildConfigValue, build_config_fingerprint,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidConfigReason};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    build_input_type_from_parsed, parsed_type_span,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
/// Resolve declaration-owned `#Config of T` metadata before the ordinary const store fold.
///
/// Top-level declarations are the bootstrap owner. The declaration-boundary path records the
/// input contract and seeds the input's stable provenance. Folded `ConstValue` metadata
/// carries that provenance into each receiving project field through the ordinary fold, so
/// this pass records no pre-fold project-field dependency facts.
#[allow(
    clippy::too_many_arguments,
    reason = "config resolution keeps the declaration, scope, services and mutable state separate"
)]
pub(super) fn resolve_config_declaration(
    declaration: &mut Declaration,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    services: &ConfigResolutionServices,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let declaration_name = path_fork.component(declaration.id);
    if let Some(qualifier) = declaration.config_qualifier.take() {
        let Some(declaration_name) = declaration_name else {
            return Err(config_expression_error(
                None,
                InvalidConfigReason::ConfigContractNameInvalid,
                qualifier.qualifier_span,
            ));
        };
        let declaration_text = string_table.resolve(declaration_name).to_owned();
        let input_name = BuildInputName::new(&declaration_text).map_err(|_| {
            config_expression_error(
                Some(declaration_name),
                InvalidConfigReason::ConfigContractNameInvalid,
                qualifier.qualifier_span,
            )
        })?;
        let Some(contract) = build_input_type_from_parsed(&qualifier.type_annotation) else {
            return Err(config_expression_error(
                Some(declaration_name),
                InvalidConfigReason::ConfigQualifierUnsupportedType,
                parsed_type_span(&qualifier.type_annotation),
            ));
        };
        // Pass the initializer as a bare `ExprId`: the view borrow must end before
        // `normalize_config_default` mutably borrows the owners for the template fold.
        let initializer = declaration.value.initializer();
        let authored_default = normalize_config_default(
            ConfigDefaultMetadata {
                field_name: declaration_name,
                default_none: qualifier.default_none,
                default_span: qualifier.qualifier_span,
                contract,
            },
            initializer,
            scope_context,
            type_interner.environment(),
            expression_builder,
            const_values,
            string_table,
        )?;
        let (contract_required, contract_default) = match &authored_default {
            AuthoredConfigDefault::Missing => (!contract.is_optional(), None),
            AuthoredConfigDefault::None { .. } => (false, None),
            AuthoredConfigDefault::Value { value, .. } => (false, Some(value.clone())),
        };
        let configured_value = if let Some(entry) = services.explicit_input(&input_name) {
            Some((
                entry.value().materialise_for(contract),
                BuildConfigValueOrigin::ExplicitInput,
                Some(entry.location().clone()),
                build_config_input_argument_index(entry.location()),
            ))
        } else {
            services.builder_global(&input_name).map(|value| {
                (
                    value.accept_for(contract),
                    BuildConfigValueOrigin::BuilderGlobal,
                    None,
                    None,
                )
            })
        };

        if let Some((value, origin, value_location, argument_index)) = configured_value {
            let value = value.map_err(|error| {
                configured_value_error(
                    error,
                    declaration_name,
                    contract,
                    qualifier.qualifier_span,
                    argument_index,
                    string_table,
                )
            })?;
            declaration.value = DeclarationValue::Initializer(
                expression_for_build_value(
                    &value,
                    contract,
                    None,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                )?
                .id(),
            );
            services.record(config_resolution_record(
                declaration_name,
                &declaration_text,
                input_name.clone(),
                contract,
                contract_required,
                contract_default.clone(),
                Some(value),
                origin,
                qualifier.qualifier_span,
                value_location,
            ));
        } else {
            match authored_default {
                AuthoredConfigDefault::Missing => {
                    if !contract.is_optional() {
                        return Err(config_expression_error(
                            Some(declaration_name),
                            InvalidConfigReason::MissingConfigInput,
                            qualifier.qualifier_span,
                        ));
                    }
                    declaration.value = DeclarationValue::Initializer(
                        option_none_expression(
                            contract,
                            qualifier.qualifier_span,
                            type_interner,
                            expression_builder,
                            const_values,
                        )?
                        .id(),
                    );
                    services.record(config_resolution_record(
                        declaration_name,
                        &declaration_text,
                        input_name.clone(),
                        contract,
                        contract_required,
                        contract_default.clone(),
                        None,
                        BuildConfigValueOrigin::DeclarationDefault,
                        qualifier.qualifier_span,
                        None,
                    ));
                }
                AuthoredConfigDefault::None { span } => {
                    if declaration.value.initializer().is_none() {
                        declaration.value = DeclarationValue::Initializer(
                            option_none_expression(
                                contract,
                                span,
                                type_interner,
                                expression_builder,
                                const_values,
                            )?
                            .id(),
                        );
                    }
                    services.record(config_resolution_record(
                        declaration_name,
                        &declaration_text,
                        input_name.clone(),
                        contract,
                        contract_required,
                        contract_default.clone(),
                        None,
                        BuildConfigValueOrigin::DeclarationDefault,
                        qualifier.qualifier_span,
                        span.map(BuildConfigValueLocation::Source),
                    ));
                }
                AuthoredConfigDefault::Value {
                    value,
                    span,
                    expression_is_optional,
                } => {
                    if contract.is_optional() && !expression_is_optional {
                        // The config owner has already validated this default as a primitive.
                        // Materialise its optional contract directly, retaining dependency provenance
                        // without moving the initializer through a fabricated temporary expression.
                        let authored_expression =
                            declaration.value.initializer_mut().ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "a validated config value default has no initializer",
                                )
                            })?;
                        let authored_expression = *authored_expression;
                        let expression = expression_for_build_value(
                            &value,
                            contract,
                            span,
                            type_interner,
                            expression_builder,
                            const_values,
                            string_table,
                        )?;
                        let expression = expression_builder.preserve_authored_metadata(
                            expression,
                            expression_builder.own(authored_expression),
                        )?;
                        declaration.value = DeclarationValue::Initializer(expression.id());
                    }
                    services.record(config_resolution_record(
                        declaration_name,
                        &declaration_text,
                        input_name.clone(),
                        contract,
                        contract_required,
                        contract_default,
                        Some(value),
                        BuildConfigValueOrigin::DeclarationDefault,
                        qualifier.qualifier_span,
                        span.map(BuildConfigValueLocation::Source),
                    ));
                }
            }
        }
        let expression_id = declaration.value.initializer().ok_or_else(|| {
            CompilerError::compiler_error("a resolved config declaration has no initializer")
        })?;
        let expression = expression_builder.view(expression_id)?;
        declaration.type_id = expression.value_type_id()?;
        declaration.diagnostic_type =
            diagnostic_type_spelling(declaration.type_id, type_interner.environment());
        declaration.value_mode = expression.value_mode();
        let provenance = expression
            .occurrence_facts()
            .map_or_else(SyntheticInterfaceProvenance::empty, |facts| {
                facts.provenance.clone()
            })
            .union(&SyntheticInterfaceProvenance::single(
                SyntheticInterfaceMemberIdentity::new(
                    SyntheticInterfaceClass::ProjectContext,
                    "source-config",
                    input_name.as_str(),
                ),
            ));
        let mut input = expression.to_completed_input(expression_builder)?;
        input.occurrence = Some(OccurrenceFacts {
            provenance: provenance.clone(),
            diagnostic_spelling: None,
        });
        declaration.value =
            DeclarationValue::Initializer(expression_builder.commit_replacement(input)?.id());
        services.record_input_provenance(input_name, provenance);
        return Ok(());
    }

    // Non-`#Config` declarations carry whatever provenance the ordinary parse already
    // attached. The const-store fold propagates that provenance into folded field metadata,
    // so no pre-fold dependency walk is needed here.
    Ok(())
}

/// A source config declaration's authored default after primitive validation.
enum AuthoredConfigDefault {
    Missing,
    None {
        span: Option<SourceSpan>,
    },
    Value {
        value: PrimitiveBuildValue,
        span: Option<SourceSpan>,
        expression_is_optional: bool,
    },
}

/// Declaration-owned identity and contract for validating an authored config default.
struct ConfigDefaultMetadata {
    field_name: StringId,
    default_none: bool,
    default_span: Option<SourceSpan>,
    contract: BuildInputType,
}

fn normalize_config_default(
    metadata: ConfigDefaultMetadata,
    expression: Option<ExprId>,
    scope_context: &ScopeContext,
    type_environment: &TypeEnvironment,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<AuthoredConfigDefault, ExpressionParseError> {
    // Snapshot every owned expression fact the read-only checks need, then drop the view
    // before `primitive_value_from_expression_id` mutably borrows the owners for the fold.
    let ConfigDefaultMetadata {
        field_name,
        default_none,
        default_span,
        contract,
    } = metadata;
    let Some(expression_id) = expression else {
        let span = default_span;
        if !default_none {
            return Ok(AuthoredConfigDefault::Missing);
        }
        if !contract.is_optional() {
            return Err(config_input_type_mismatch(
                Some(field_name),
                "None",
                contract,
                span,
                None,
                string_table,
            ));
        }
        return Ok(AuthoredConfigDefault::None { span });
    };
    let (span, value_type_id, snapshot_const) = {
        let snapshot = expression_builder.view(expression_id)?;
        let span = snapshot.span();
        let value_type_id = snapshot.value_type_id()?;
        let snapshot_const = match snapshot.operation() {
            ExpressionOperationView::Constant(id) => Some(id),
            _ => None,
        };
        (span, value_type_id, snapshot_const)
    };
    let is_option_none = snapshot_const.is_some_and(|id| {
        matches!(
            const_values.payload(id),
            Some(ConstValuePayload::OptionNone)
        )
    });

    if is_option_none {
        if !contract.is_optional() {
            return Err(config_input_type_mismatch(
                Some(field_name),
                "None",
                contract,
                span,
                None,
                string_table,
            ));
        }
        return Ok(AuthoredConfigDefault::None { span });
    }

    let Some(value) = primitive_value_from_expression_id(
        expression_id,
        scope_context,
        expression_builder,
        const_values,
        string_table,
    )?
    else {
        return Err(config_input_type_mismatch(
            Some(field_name),
            "non-primitive",
            contract,
            span,
            None,
            string_table,
        ));
    };
    if !contract.accepts_primitive(value.primitive_type()) {
        return Err(config_input_type_mismatch(
            Some(field_name),
            value.primitive_type().name(),
            contract,
            span,
            None,
            string_table,
        ));
    }
    Ok(AuthoredConfigDefault::Value {
        value,
        span,
        expression_is_optional: type_environment.option_inner_type(value_type_id).is_some(),
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "resolution record keeps field identity and text, input name and contract, required/default/value state, origin, and qualifier/value spans as separate inputs"
)]
fn config_resolution_record(
    field_name: StringId,
    field_text: &str,
    input_name: BuildInputName,
    contract: BuildInputType,
    required: bool,
    default: Option<PrimitiveBuildValue>,
    value: Option<PrimitiveBuildValue>,
    origin: BuildConfigValueOrigin,
    qualifier_span: Option<SourceSpan>,
    value_location: Option<BuildConfigValueLocation>,
) -> ConfigResolutionRecord {
    let fingerprint = build_config_fingerprint(field_text, contract, value.as_ref());
    ConfigResolutionRecord {
        field_name,
        input_name,
        contract,
        required,
        default,
        value,
        origin,
        fingerprint,
        qualifier_span,
        value_location,
    }
}

fn config_expression_error(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    span: Option<SourceSpan>,
) -> ExpressionParseError {
    CompilerDiagnostic::invalid_config_reason(key, reason, span).into()
}

fn config_input_type_mismatch(
    key: Option<StringId>,
    provided: &str,
    contract: BuildInputType,
    span: Option<SourceSpan>,
    provided_argument_index: Option<usize>,
    string_table: &mut StringTable,
) -> ExpressionParseError {
    config_expression_error(
        key,
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided: string_table.intern(provided),
            expected: string_table.intern(&config_type_name(contract)),
            provided_argument_index,
        },
        span,
    )
}

/// Map one contract rejection of a configured value onto the declaration's diagnostic.
///
/// WHAT: a type mismatch keeps the `#Config` type diagnostic, with the command argument position
///       when the value came from the command. A command literal outside its scalar's range keeps
///       that position too; any other out-of-range literal reports the number-literal diagnostic.
/// WHY:  the declaration qualifier is the only source span this lane owns for a configured value.
fn configured_value_error(
    error: ContractValueError,
    key: StringId,
    contract: BuildInputType,
    span: Option<SourceSpan>,
    provided_argument_index: Option<usize>,
    string_table: &mut StringTable,
) -> ExpressionParseError {
    match error {
        ContractValueError::TypeMismatch { provided } => config_input_type_mismatch(
            Some(key),
            provided.name(),
            contract,
            span,
            provided_argument_index,
            string_table,
        ),
        ContractValueError::LiteralOutOfRange { text, reason } => {
            let literal = string_table.intern(&text);
            match provided_argument_index {
                Some(index) => config_expression_error(
                    Some(key),
                    InvalidConfigReason::ConfigInputOutOfRange {
                        literal,
                        reason,
                        provided_argument_index: Some(index),
                    },
                    span,
                ),
                None => CompilerDiagnostic::invalid_number_literal(literal, reason, span).into(),
            }
        }
    }
}

fn build_config_input_argument_index(location: &BuildConfigValueLocation) -> Option<usize> {
    match location {
        BuildConfigValueLocation::Command(location) => Some(location.argument_index()),
        BuildConfigValueLocation::Source(_) => None,
    }
}

fn config_type_name(contract: BuildInputType) -> String {
    let primitive = contract.primitive().name();
    if contract.is_optional() {
        format!("{primitive}?")
    } else {
        primitive.to_owned()
    }
}

fn primitive_type_id(primitive: PrimitiveBuildInputType) -> TypeId {
    match primitive {
        PrimitiveBuildInputType::String => builtin_type_ids::STRING,
        // Each admitted fixed scalar projects onto its own exact language identity.
        PrimitiveBuildInputType::Fixed(scalar) => builtin_type_ids::fixed_scalar(scalar),
        PrimitiveBuildInputType::Bool => builtin_type_ids::BOOL,
        PrimitiveBuildInputType::Char => builtin_type_ids::CHAR,
    }
}

fn option_none_expression(
    contract: BuildInputType,
    span: Option<SourceSpan>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<OwnedExpr, ExpressionParseError> {
    Ok(expression_builder.option_none_with_type_id(
        const_values,
        primitive_type_id(contract.primitive()),
        type_interner.environment_mut_for_derived_types(),
        span,
    )?)
}

/// Materialise a config value in the receiving phase's constant and expression owners.
pub(crate) fn expression_for_resolved_build_config_value(
    resolved: &ResolvedBuildConfigValue,
    span: Option<SourceSpan>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<OwnedExpr, ExpressionParseError> {
    let expression = match resolved.value() {
        Some(value) => expression_for_build_value(
            value,
            resolved.value_type(),
            span,
            type_interner,
            expression_builder,
            const_values,
            string_table,
        )?,
        None => option_none_expression(
            resolved.value_type(),
            span,
            type_interner,
            expression_builder,
            const_values,
        )?,
    };
    let mut input = expression_builder
        .view_owned(expression)?
        .to_completed_input(expression_builder)?;
    input.occurrence = Some(OccurrenceFacts {
        provenance: SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "source-config",
            resolved.name().as_str(),
        )),
        diagnostic_spelling: None,
    });
    Ok(expression_builder.commit_replacement(input)?)
}

fn expression_for_build_value(
    value: &PrimitiveBuildValue,
    contract: BuildInputType,
    span: Option<SourceSpan>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<OwnedExpr, ExpressionParseError> {
    let payload = match value {
        PrimitiveBuildValue::String(text) => {
            ConstValuePayload::String(ConstStringValue::Text(string_table.intern(text)))
        }
        PrimitiveBuildValue::Fixed(value) => ConstValuePayload::FixedScalar(*value),
        PrimitiveBuildValue::Bool(value) => ConstValuePayload::Bool(*value),
        PrimitiveBuildValue::Char(value) => ConstValuePayload::Char(*value),
    };
    let inner_type = primitive_type_id(contract.primitive());
    let inner = const_values.insert_resolved(inner_type, payload, span)?;
    let (value, type_id) = if contract.is_optional() {
        let type_id = type_interner
            .environment_mut_for_derived_types()
            .intern_option(inner_type);
        (
            const_values.insert_resolved(type_id, ConstValuePayload::OptionSome(inner), span)?,
            type_id,
        )
    } else {
        (inner, inner_type)
    };
    Ok(expression_builder.constant(
        value,
        type_id,
        span,
        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
    )?)
}

fn primitive_value_from_expression_id(
    expression: ExprId,
    scope_context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<Option<PrimitiveBuildValue>, ExpressionParseError> {
    // Copy the operation's owned child identity first: the view borrows the builder and
    // must end before the recursive call reborrows it mutably. Only `Constant` keeps a
    // direct borrow; `Coerce`/`Template` snapshot to owned IDs/values.
    enum PrimitiveStep {
        Constant(crate::compiler_frontend::ast::const_values::store::ConstValueId),
        Coerce(ExprId),
        Template(crate::compiler_frontend::ast::templates::template::Template),
        Other,
    }
    let step = {
        let view = expression_builder.view(expression)?;
        match view.operation() {
            ExpressionOperationView::Constant(id) => PrimitiveStep::Constant(id),
            ExpressionOperationView::Coerce { value, .. } => PrimitiveStep::Coerce(value.id()),
            ExpressionOperationView::Template(template) => {
                PrimitiveStep::Template(template.clone())
            }
            _ => PrimitiveStep::Other,
        }
    };
    match step {
        PrimitiveStep::Constant(id) => primitive_value_from_payload(id, const_values, string_table),
        PrimitiveStep::Coerce(inner) => primitive_value_from_expression_id(
            inner,
            scope_context,
            expression_builder,
            const_values,
            string_table,
        ),
        PrimitiveStep::Template(template) => primitive_value_from_template(
            &template,
            scope_context,
            expression_builder,
            const_values,
            string_table,
        ),
        PrimitiveStep::Other => Ok(None),
    }
}
fn primitive_value_from_payload(
    id: crate::compiler_frontend::ast::const_values::store::ConstValueId,
    const_values: &ConstValueStore,
    string_table: &StringTable,
) -> Result<Option<PrimitiveBuildValue>, ExpressionParseError> {
    Ok(
        match const_values.payload(id).ok_or_else(|| {
            CompilerError::compiler_error("config constant ID is outside its owner")
        })? {
            ConstValuePayload::String(ConstStringValue::Text(value)) => Some(
                PrimitiveBuildValue::String(string_table.resolve(*value).to_owned()),
            ),
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)) => {
                primitive_value_from_string_pieces(pieces, string_table)
                    .map(PrimitiveBuildValue::String)
            }
            ConstValuePayload::FixedScalar(value) => Some(PrimitiveBuildValue::Fixed(*value)),
            ConstValuePayload::Bool(value) => Some(PrimitiveBuildValue::Bool(*value)),
            ConstValuePayload::Char(value) => Some(PrimitiveBuildValue::Char(*value)),
            ConstValuePayload::OptionSome(value) => {
                return primitive_value_from_payload(*value, const_values, string_table);
            }
            _ => None,
        },
    )
}

fn primitive_value_from_template(
    template: &crate::compiler_frontend::ast::templates::template::Template,
    scope_context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<Option<PrimitiveBuildValue>, ExpressionParseError> {
    let reference = template.tir_reference;
    // preparation borrows the owners shared; the fold needs `&mut` scratch borrows. The
    // `template` view borrow ends here (only the `Copy` TIR reference survives), and the
    // preparation's shared borrows must also end before the fold reborrows mutably.
    let preparation = {
        let store = scope_context.template_ir_store.borrow();
        let view = TirView::with_minimum_phase(
            &store,
            reference.root,
            reference.phase,
            TemplateTirPhase::Composed,
            reference.context,
        )
        .map_err(ExpressionParseError::from)?;
        prepare_tir_view(
            &view,
            expression_builder,
            const_values,
            TemplatePreparationMode::Value,
        )
        .map_err(ExpressionParseError::from)?
    };
    if !matches!(preparation.outcome, TemplatePreparationOutcome::Foldable)
        || !matches!(
            preparation.facts.final_value_kind,
            TemplateConstValueKind::RenderableString
        )
    {
        return Ok(None);
    }
    let mut fold_context =
        scope_context.new_tir_fold_context(string_table, expression_builder, const_values);
    let store = scope_context.template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )
    .map_err(ExpressionParseError::from)?;
    let fold_result = fold_prepared_template(&preparation, view, &mut fold_context)
        .map_err(ExpressionParseError::from)?;
    match fold_result.emission {
        TemplateEmission::NoOutput => Ok(Some(PrimitiveBuildValue::String(String::new()))),
        TemplateEmission::Output(value) => {
            Ok(primitive_value_from_const_string(value, string_table))
        }
    }
}

fn primitive_value_from_const_string(
    value: ConstStringValue,
    string_table: &StringTable,
) -> Option<PrimitiveBuildValue> {
    match value {
        ConstStringValue::Text(value) => Some(PrimitiveBuildValue::String(
            string_table.resolve(value).to_owned(),
        )),
        ConstStringValue::Pieces(pieces) => {
            primitive_value_from_string_pieces(&pieces, string_table)
                .map(PrimitiveBuildValue::String)
        }
    }
}

fn primitive_value_from_string_pieces(
    pieces: &[ConstStringPiece],
    string_table: &StringTable,
) -> Option<String> {
    let mut value = String::new();
    for piece in pieces {
        let ConstStringPiece::Text(text) = piece else {
            return None;
        };
        value.push_str(string_table.resolve(*text));
    }
    Some(value)
}

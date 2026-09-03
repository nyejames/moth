//! AST direct-project configuration resolution.
//!
//! WHAT: resolves declaration-owned `#Config of T` metadata on direct project fields and records
//! the resulting build-input facts.
//! WHY: configuration validation and primitive materialisation are a focused semantic phase,
//! separate from the ordinary top-level constant session and its compile-time fold.

use crate::builder_surface::config_schema::{ConfigFieldShape, ProjectFieldConfigPolicy};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TirView,
    fold_prepared_template, prepare_tir_view,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::build_config::{
    BuildConfigValueLocation, BuildConfigValueOrigin, BuildInputName, BuildInputType,
    ConfigResolutionRecord, ConfigResolutionServices, PrimitiveBuildInputType, PrimitiveBuildValue,
    ResolvedBuildConfigValue, build_config_fingerprint,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidConfigReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::{DataType, diagnostic_type_spelling};
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    build_input_type_from_parsed, build_input_type_name, parsed_type_location,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};

/// Resolve direct-project `#Config of T` field metadata before the ordinary const store fold.
pub(super) fn resolve_direct_project_config_qualifiers(
    declaration: &mut Declaration,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    services: &ConfigResolutionServices,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    if let Some(qualifier) = declaration.config_qualifier.take() {
        return Err(config_expression_error(
            declaration.id.name(),
            InvalidConfigReason::ConfigQualifierInvalidProjectPlacement,
            qualifier.qualifier_location,
        ));
    }

    let is_project = declaration
        .id
        .name()
        .is_some_and(|name| string_table.resolve(name) == "project");
    let ExpressionKind::AnonymousConstRecord { fields } = &mut declaration.value.kind else {
        return Ok(());
    };

    for field in fields {
        let field_name = field.id.name();
        if let Some(qualifier) = field.config_qualifier.take() {
            let Some(field_name) = field_name else {
                return Err(config_expression_error(
                    None,
                    InvalidConfigReason::ConfigQualifierInvalidProjectPlacement,
                    qualifier.qualifier_location,
                ));
            };
            if !is_project {
                return Err(config_expression_error(
                    Some(field_name),
                    InvalidConfigReason::ConfigQualifierInvalidProjectPlacement,
                    qualifier.qualifier_location,
                ));
            }

            let field_text = string_table.resolve(field_name).to_owned();
            if services.project_field_policy(&field_text) != ProjectFieldConfigPolicy::Configurable
            {
                return Err(config_expression_error(
                    Some(field_name),
                    InvalidConfigReason::ConfigQualifierFixedField,
                    qualifier.qualifier_location,
                ));
            }
            let Some(contract) = build_input_type_from_parsed(&qualifier.type_annotation) else {
                return Err(config_expression_error(
                    Some(field_name),
                    InvalidConfigReason::ConfigQualifierUnsupportedType,
                    parsed_type_location(&qualifier.type_annotation),
                ));
            };
            if let Some(shape) = services.project_field_shape(&field_text)
                && !project_shape_accepts_contract(shape, contract)
            {
                let declared = string_table.intern(&build_input_type_name(contract));
                let expected = string_table.intern(&shape.describe());
                return Err(config_expression_error(
                    Some(field_name),
                    InvalidConfigReason::ConfigQualifierSchemaTypeMismatch { declared, expected },
                    qualifier.qualifier_location,
                ));
            }
            let input_name = BuildInputName::new(&field_text).map_err(|_| {
                config_expression_error(
                    Some(field_name),
                    InvalidConfigReason::ConfigContractNameInvalid,
                    qualifier.qualifier_location.clone(),
                )
            })?;
            let authored_default = normalize_config_default(
                field_name,
                &field.value,
                qualifier.default_none,
                contract,
                scope_context,
                type_interner.environment(),
                string_table,
            )?;
            let qualifier_location = qualifier.qualifier_location.clone();
            let (contract_required, contract_default) = match &authored_default {
                AuthoredConfigDefault::Missing => (!contract.is_optional(), None),
                AuthoredConfigDefault::None { .. } => (false, None),
                AuthoredConfigDefault::Value { value, .. } => (false, Some(value.clone())),
            };
            let configured_value = if let Some(entry) = services.explicit_input(&input_name) {
                Some((
                    entry.value(),
                    BuildConfigValueOrigin::ExplicitInput,
                    Some(entry.location().clone()),
                    build_config_input_argument_index(entry.location()),
                ))
            } else {
                services
                    .builder_global(&input_name)
                    .map(|value| (value, BuildConfigValueOrigin::BuilderGlobal, None, None))
            };

            if let Some((value, origin, value_location, argument_index)) = configured_value {
                if !contract.accepts_primitive(value.primitive_type()) {
                    return Err(config_input_type_mismatch(
                        Some(field_name),
                        value.primitive_type().name(),
                        contract,
                        qualifier_location.clone(),
                        argument_index,
                        string_table,
                    ));
                }
                field.value = expression_for_build_value(
                    value,
                    contract,
                    qualifier_location.clone(),
                    type_interner,
                    string_table,
                );
                services.record(config_resolution_record(
                    field_name,
                    &field_text,
                    contract,
                    contract_required,
                    contract_default.clone(),
                    Some(value.clone()),
                    origin,
                    qualifier_location,
                    value_location,
                ));
            } else {
                match authored_default {
                    AuthoredConfigDefault::Missing => {
                        if !contract.is_optional() {
                            return Err(config_expression_error(
                                Some(field_name),
                                InvalidConfigReason::MissingConfigInput,
                                qualifier_location.clone(),
                            ));
                        }

                        let value_location = field.value.location.clone();
                        field.value =
                            option_none_expression(contract, value_location, type_interner);
                        services.record(config_resolution_record(
                            field_name,
                            &field_text,
                            contract,
                            contract_required,
                            contract_default.clone(),
                            None,
                            BuildConfigValueOrigin::DeclarationDefault,
                            qualifier_location.clone(),
                            None,
                        ));
                    }
                    AuthoredConfigDefault::None { location } => {
                        if matches!(field.value.kind, ExpressionKind::NoValue) {
                            field.value =
                                option_none_expression(contract, location.clone(), type_interner);
                        }
                        services.record(config_resolution_record(
                            field_name,
                            &field_text,
                            contract,
                            contract_required,
                            contract_default.clone(),
                            None,
                            BuildConfigValueOrigin::DeclarationDefault,
                            qualifier_location.clone(),
                            Some(BuildConfigValueLocation::Source(location)),
                        ));
                    }
                    AuthoredConfigDefault::Value {
                        value,
                        location,
                        expression_is_optional,
                    } => {
                        if contract.is_optional() && !expression_is_optional {
                            let inner = std::mem::replace(
                                &mut field.value,
                                Expression::no_value(
                                    qualifier_location.clone(),
                                    DataType::Inferred,
                                    crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                                ),
                            );
                            field.value =
                                expression_for_optional_default(inner, contract, type_interner);
                        }
                        services.record(config_resolution_record(
                            field_name,
                            &field_text,
                            contract,
                            contract_required,
                            contract_default,
                            Some(value),
                            BuildConfigValueOrigin::DeclarationDefault,
                            qualifier_location,
                            Some(BuildConfigValueLocation::Source(location)),
                        ));
                    }
                }
            }
            field.value.synthetic_interface_provenance =
                field.value.synthetic_interface_provenance.union(
                    &SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
                        SyntheticInterfaceClass::ProjectContext,
                        "project",
                        field_text,
                    )),
                );
        }
    }
    Ok(())
}

/// A direct project's authored default after config-specific primitive validation.
enum AuthoredConfigDefault {
    Missing,
    None {
        location: SourceLocation,
    },
    Value {
        value: PrimitiveBuildValue,
        location: SourceLocation,
        expression_is_optional: bool,
    },
}

fn normalize_config_default(
    field_name: StringId,
    expression: &Expression,
    default_none: bool,
    contract: BuildInputType,
    scope_context: &ScopeContext,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
) -> Result<AuthoredConfigDefault, ExpressionParseError> {
    let location = expression.location.clone();
    if matches!(expression.kind, ExpressionKind::NoValue) {
        if !default_none {
            return Ok(AuthoredConfigDefault::Missing);
        }
        if !contract.is_optional() {
            return Err(config_input_type_mismatch(
                Some(field_name),
                "None",
                contract,
                location,
                None,
                string_table,
            ));
        }
        return Ok(AuthoredConfigDefault::None { location });
    }

    if matches!(expression.kind, ExpressionKind::OptionNone) {
        if !contract.is_optional() {
            return Err(config_input_type_mismatch(
                Some(field_name),
                "None",
                contract,
                location,
                None,
                string_table,
            ));
        }
        return Ok(AuthoredConfigDefault::None { location });
    }

    let Some(value) = primitive_value_from_expression(expression, scope_context, string_table)?
    else {
        return Err(config_input_type_mismatch(
            Some(field_name),
            "non-primitive",
            contract,
            location,
            None,
            string_table,
        ));
    };
    if !contract.accepts_primitive(value.primitive_type()) {
        return Err(config_input_type_mismatch(
            Some(field_name),
            value.primitive_type().name(),
            contract,
            location.clone(),
            None,
            string_table,
        ));
    }

    Ok(AuthoredConfigDefault::Value {
        value,
        location,
        expression_is_optional: type_environment
            .option_inner_type(expression.type_id)
            .is_some(),
    })
}

#[allow(clippy::too_many_arguments)]
fn config_resolution_record(
    field_name: StringId,
    field_text: &str,
    contract: BuildInputType,
    required: bool,
    default: Option<PrimitiveBuildValue>,
    value: Option<PrimitiveBuildValue>,
    origin: BuildConfigValueOrigin,
    qualifier_location: SourceLocation,
    value_location: Option<BuildConfigValueLocation>,
) -> ConfigResolutionRecord {
    let fingerprint = build_config_fingerprint(field_text, contract, value.as_ref());
    ConfigResolutionRecord {
        field_name,
        contract,
        required,
        default,
        value,
        origin,
        fingerprint,
        qualifier_location,
        value_location,
    }
}

fn config_expression_error(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    location: SourceLocation,
) -> ExpressionParseError {
    CompilerDiagnostic::invalid_config_reason(key, reason, location).into()
}

fn config_input_type_mismatch(
    key: Option<StringId>,
    provided: &str,
    contract: BuildInputType,
    location: SourceLocation,
    provided_argument_index: Option<usize>,
    string_table: &mut StringTable,
) -> ExpressionParseError {
    config_expression_error(
        key,
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided: string_table.intern(provided),
            expected: string_table.intern(&build_input_type_name(contract)),
            provided_argument_index,
        },
        location,
    )
}

fn build_config_input_argument_index(location: &BuildConfigValueLocation) -> Option<usize> {
    match location {
        BuildConfigValueLocation::Command(location) => Some(location.argument_index()),
        BuildConfigValueLocation::Source(_) => None,
    }
}

fn project_shape_accepts_contract(shape: &ConfigFieldShape, contract: BuildInputType) -> bool {
    match shape {
        ConfigFieldShape::Optional(inner) => project_shape_primitive(inner)
            .is_some_and(|primitive| primitive == contract.primitive()),
        _ => project_shape_primitive(shape)
            .is_some_and(|primitive| primitive == contract.primitive() && !contract.is_optional()),
    }
}

fn project_shape_primitive(shape: &ConfigFieldShape) -> Option<PrimitiveBuildInputType> {
    match shape {
        ConfigFieldShape::String => Some(PrimitiveBuildInputType::String),
        ConfigFieldShape::Int => Some(PrimitiveBuildInputType::Int),
        ConfigFieldShape::Float => Some(PrimitiveBuildInputType::Float),
        ConfigFieldShape::Bool => Some(PrimitiveBuildInputType::Bool),
        ConfigFieldShape::Char => Some(PrimitiveBuildInputType::Char),
        ConfigFieldShape::Optional(_)
        | ConfigFieldShape::Record(_)
        | ConfigFieldShape::Collection(_) => None,
    }
}

fn primitive_type_id(primitive: PrimitiveBuildInputType) -> TypeId {
    match primitive {
        PrimitiveBuildInputType::String => builtin_type_ids::STRING,
        PrimitiveBuildInputType::Int => builtin_type_ids::INT,
        PrimitiveBuildInputType::Float => builtin_type_ids::FLOAT,
        PrimitiveBuildInputType::Bool => builtin_type_ids::BOOL,
        PrimitiveBuildInputType::Char => builtin_type_ids::CHAR,
    }
}

fn option_none_expression(
    contract: BuildInputType,
    location: SourceLocation,
    type_interner: &mut AstTypeInterner<'_>,
) -> Expression {
    let inner_type_id = primitive_type_id(contract.primitive());
    let inner_diagnostic = diagnostic_type_spelling(inner_type_id, type_interner.environment());
    Expression::option_none_with_type_id(
        inner_type_id,
        inner_diagnostic,
        type_interner.environment_mut_for_derived_types(),
        location,
    )
}

/// Materialize one resolved source `#Config` value as an ordinary constant expression.
///
/// Source config values enter the existing declaration/constant-folding path through this helper;
/// no Config-specific AST or HIR value is created.
pub(crate) fn expression_for_resolved_build_config_value(
    resolved: &ResolvedBuildConfigValue,
    location: SourceLocation,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Expression {
    let mut expression = match resolved.value() {
        Some(value) => expression_for_build_value(
            value,
            resolved.value_type(),
            location,
            type_interner,
            string_table,
        ),
        None => {
            debug_assert!(
                resolved.value_type().is_optional(),
                "only optional source config contracts can resolve to absence"
            );
            option_none_expression(resolved.value_type(), location, type_interner)
        }
    };
    expression.synthetic_interface_provenance =
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "source-config",
            resolved.name().as_str(),
        ));
    expression
}

fn expression_for_build_value(
    value: &PrimitiveBuildValue,
    contract: BuildInputType,
    location: SourceLocation,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Expression {
    let inner = match value {
        PrimitiveBuildValue::String(text) => Expression::string_slice(
            string_table.intern(text),
            location.clone(),
            crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
        ),
        PrimitiveBuildValue::Int(value) => Expression::int(
            *value,
            location.clone(),
            crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
        ),
        PrimitiveBuildValue::Float(value) => Expression::float(
            value.value(),
            location.clone(),
            crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
        ),
        PrimitiveBuildValue::Bool(value) => Expression::bool(
            *value,
            location.clone(),
            crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
        ),
        PrimitiveBuildValue::Char(value) => Expression::char(
            *value,
            location.clone(),
            crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
        ),
    };
    if contract.is_optional() {
        expression_for_optional_default(inner, contract, type_interner)
    } else {
        inner
    }
}

fn expression_for_optional_default(
    inner: Expression,
    contract: BuildInputType,
    type_interner: &mut AstTypeInterner<'_>,
) -> Expression {
    let inner_type_id = primitive_type_id(contract.primitive());
    let option_type_id = type_interner
        .environment_mut_for_derived_types()
        .intern_option(inner_type_id);
    let inner_diagnostic = diagnostic_type_spelling(inner_type_id, type_interner.environment());
    let location = inner.location.clone();
    Expression::new(
        ExpressionKind::Coerced {
            value: Box::new(inner),
            to_type: option_type_id,
        },
        location,
        option_type_id,
        DataType::Option(Box::new(inner_diagnostic)),
        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
    )
}

fn primitive_value_from_expression(
    expression: &Expression,
    scope_context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<Option<PrimitiveBuildValue>, ExpressionParseError> {
    match &expression.kind {
        ExpressionKind::StringSlice(value) => Ok(Some(PrimitiveBuildValue::String(
            string_table.resolve(*value).to_owned(),
        ))),
        ExpressionKind::StructuralString { pieces } => {
            Ok(primitive_value_from_string_pieces(pieces, string_table)
                .map(PrimitiveBuildValue::String))
        }
        ExpressionKind::Int(value) => Ok(Some(PrimitiveBuildValue::Int(*value))),
        ExpressionKind::Float(value) => Ok(PrimitiveBuildValue::float(*value).ok()),
        ExpressionKind::Bool(value) => Ok(Some(PrimitiveBuildValue::Bool(*value))),
        ExpressionKind::Char(value) => Ok(Some(PrimitiveBuildValue::Char(*value))),
        ExpressionKind::Coerced { value, .. } => {
            primitive_value_from_expression(value, scope_context, string_table)
        }
        ExpressionKind::Template(template) => {
            primitive_value_from_template(template, scope_context, string_table)
        }
        _ => Ok(None),
    }
}

fn primitive_value_from_template(
    template: &crate::compiler_frontend::ast::templates::template::Template,
    scope_context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<Option<PrimitiveBuildValue>, ExpressionParseError> {
    let reference = template.tir_reference;
    let store = scope_context.template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )
    .map_err(ExpressionParseError::from)?;
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .map_err(ExpressionParseError::from)?;
    if !matches!(preparation.outcome, TemplatePreparationOutcome::Foldable)
        || !matches!(
            preparation.facts.final_value_kind,
            TemplateConstValueKind::RenderableString
        )
    {
        return Ok(None);
    }

    let mut fold_context = scope_context.new_tir_fold_context(string_table);
    let fold_result = fold_prepared_template(&preparation, view, &mut fold_context)
        .map_err(ExpressionParseError::from)?;
    match fold_result.emission {
        crate::compiler_frontend::ast::templates::template_folding::TemplateEmission::NoOutput => {
            Ok(Some(PrimitiveBuildValue::String(String::new())))
        }
        crate::compiler_frontend::ast::templates::template_folding::TemplateEmission::Output(
            value,
        ) => Ok(primitive_value_from_const_string(value, string_table)),
        crate::compiler_frontend::ast::templates::template_folding::TemplateEmission::Break(_)
        | crate::compiler_frontend::ast::templates::template_folding::TemplateEmission::Continue(
            _,
        ) => Ok(None),
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

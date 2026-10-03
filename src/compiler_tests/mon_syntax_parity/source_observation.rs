//! Typed observations of compiler constants, expressions and parsed call arguments.
//!
//! WHAT: adapts existing compiler AST and folded-store values for comparison with MON's public
//! `Value`; no parser, semantic stage or production value model is introduced here.
//! WHY: positive parity cases need an independent typed assertion instead of merely comparing two
//! acceptance results.

use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::profile::NumericProfile;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::{Ast, AstBuildResult};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, TypeConstructor, TypeId,
};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_signature_by_name,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result_with_profile;
use crate::mon::Value;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

/// Source-side semantic identity retained next to the folded value payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SourceTypeIdentity {
    Bool,
    Int,
    Float,
    String,
    Char,
    None,
    Number(u16),
    FixedScalar(FixedScalar),
    Struct(String),
    Choice(String),
    Collection,
    Map,
    Option,
    AnonymousRecord,
    Other(String),
}

/// One compiler-owned value projected only far enough for the parity assertion.
#[derive(Debug, Clone)]
pub(super) struct ObservedSourceValue {
    pub(super) type_identity: SourceTypeIdentity,
    pub(super) kind: ObservedSourceKind,
}

#[derive(Debug, Clone)]
pub(super) enum ObservedSourceKind {
    Int(i64),
    Float(f64),
    Number(NumberValue),
    FixedScalar(FixedScalarValue),
    Bool(bool),
    Char(char),
    String(String),
    Collection(Vec<ObservedSourceValue>),
    Map(Vec<(ObservedSourceValue, ObservedSourceValue)>),
    Record {
        nominal_name: Option<String>,
        fields: Vec<ObservedSourceField>,
    },
    Choice {
        nominal_name: String,
        variant_name: String,
        fields: Vec<ObservedSourceField>,
    },
    OptionSome(Box<ObservedSourceValue>),
    OptionNone,
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub(super) struct ObservedSourceField {
    pub(super) name: String,
    pub(super) value: ObservedSourceValue,
}

/// Extra expected identity where MON's value carrier intentionally omits that identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SourceIdentityExpectation {
    pub(super) path: String,
    pub(super) kind: SourceIdentityExpectationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SourceIdentityExpectationKind {
    Nominal(String),
    Decimal { coefficient: String, scale: u16 },
}

/// One supplied ordinary-call argument and the declaration-order slot selected by parsing.
#[derive(Debug, Clone)]
pub(super) struct ObservedCallArgument {
    pub(super) parameter_slot: usize,
    pub(super) parameter_name: String,
    pub(super) value: ObservedSourceValue,
}

struct ObservationContext<'a> {
    ast: &'a Ast,
    path_fork: &'a PathInternerFork,
    string_table: &'a StringTable,
}

/// Observe the `data` value from its folded module constant or its start-body declaration.
///
/// The latter path is needed only for source fixtures whose runtime map literal cannot enter the
/// compiler's folded-constant store. Both paths read existing typed compiler structures.
pub(super) fn observe_data_value(
    build_result: &AstBuildResult,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Option<ObservedSourceValue> {
    let context = ObservationContext {
        ast: &build_result.ast,
        path_fork,
        string_table,
    };

    let data_constant = build_result
        .ast
        .const_values
        .iter_module_constant_views()
        .find(|row| path_component_text(*row.path, &context) == "data");
    if let Some(row) = data_constant {
        return Some(observe_stored_value(
            &build_result.ast.const_values,
            row.id,
            &context,
        ));
    }

    let start_body = function_body_by_name(
        &build_result.ast,
        path_fork,
        string_table,
        IMPLICIT_START_FUNC_NAME,
    );
    start_body.iter().find_map(|node| match &node.kind {
        NodeKind::VariableDeclaration(declaration)
            if path_component_text(declaration.id, &context) == "data" =>
        {
            Some(observe_expression(&declaration.value, &context))
        }
        _ => None,
    })
}

/// Parse a source-only start-body call and retain argument values plus the parser-selected slots.
pub(super) fn observe_start_body_call_arguments(
    source: &str,
    profile: NumericProfile,
    function_name: &str,
) -> Vec<ObservedCallArgument> {
    let (build_result, path_fork, string_table) = parse_single_file_ast_build_result_with_profile(
        source, profile,
    )
    .unwrap_or_else(|diagnostic| {
        panic!(
            "source call fixture could not be compiled: {} {:?}",
            diagnostic.identity().code,
            diagnostic.payload
        )
    });
    let ast = &build_result.ast;
    let context = ObservationContext {
        ast,
        path_fork: &path_fork,
        string_table: &string_table,
    };
    let start_body =
        function_body_by_name(ast, &path_fork, &string_table, IMPLICIT_START_FUNC_NAME);
    let arguments = start_body
        .iter()
        .find_map(|node| {
            start_body_expression(node).and_then(|expression| {
                call_arguments_for_function(expression, function_name, &context)
            })
        })
        .unwrap_or_else(|| panic!("start body did not contain a call to '{function_name}'"));
    let signature = function_signature_by_name(ast, &path_fork, &string_table, function_name);

    arguments
        .iter()
        .map(|argument| {
            let parameter_slot = argument
                .parameter_slot
                .map(|slot| slot.index())
                .unwrap_or_else(|| {
                    panic!(
                        "call to '{function_name}' did not retain a parameter slot for argument {:?}",
                        argument.value.kind
                    )
                });
            let parameter = signature
                .parameters
                .get(parameter_slot)
                .unwrap_or_else(|| {
                    panic!(
                        "call to '{function_name}' retained out-of-range parameter slot {parameter_slot}"
                    )
                });
            let parameter_name = path_component_text(parameter.id, &context);

            if let Some(target_parameter) = argument.target_param {
                let target_name = resolve_string(target_parameter, &context);
                assert_eq!(
                    target_name, parameter_name,
                    "named call argument target did not agree with its retained slot"
                );
            }

            ObservedCallArgument {
                parameter_slot,
                parameter_name,
                value: observe_expression(&argument.value, &context),
            }
        })
        .collect()
}

/// Compare a compiler observation to MON's independent expected literal and declared option layer.
pub(super) fn assert_source_matches(
    expected: &Value,
    observed: &ObservedSourceValue,
    case: &str,
    path: &str,
    identity_expectations: &[SourceIdentityExpectation],
    optional_paths: &[String],
) {
    // MON only decodes `none` through an optional receiver, so an expected `None` always demands
    // the optional source layer. Present optional values have no MON marker and need an explicit
    // `with_source_optional` path: the public schema exposes no shape for deriving them.
    let expects_optional = expected == &Value::None
        || optional_paths
            .iter()
            .any(|optional_path| optional_path == path);
    let observed_optional = matches!(
        observed.kind,
        ObservedSourceKind::OptionSome(_) | ObservedSourceKind::OptionNone
    );

    if expects_optional != observed_optional {
        source_mismatch_message(
            case,
            path,
            &format!(
                "expected optional layer: {expects_optional}, observed {:?} with identity {:?}",
                observed.kind, observed.type_identity
            ),
        );
    }
    if observed_optional && observed.type_identity != SourceTypeIdentity::Option {
        source_mismatch_message(
            case,
            path,
            &format!(
                "optional value carried non-optional source identity {:?}",
                observed.type_identity
            ),
        );
    }
    match &observed.kind {
        ObservedSourceKind::OptionSome(inner) => assert_source_matches_value(
            expected,
            inner,
            case,
            path,
            identity_expectations,
            optional_paths,
        ),
        ObservedSourceKind::OptionNone => {
            if expected != &Value::None {
                source_mismatch(case, path, expected, observed);
            }
        }
        _ => assert_source_matches_value(
            expected,
            observed,
            case,
            path,
            identity_expectations,
            optional_paths,
        ),
    }
}

fn assert_source_matches_value(
    expected: &Value,
    observed: &ObservedSourceValue,
    case: &str,
    path: &str,
    identity_expectations: &[SourceIdentityExpectation],
    optional_paths: &[String],
) {
    let identity_expectation = identity_expectations
        .iter()
        .find(|expectation| expectation.path == path);

    match expected {
        // `assert_source_matches` owns the optional layer, so `None` never reaches a payload.
        Value::None => source_mismatch(case, path, expected, observed),
        Value::Bool(expected_value) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(case, path, expected, observed, SourceTypeIdentity::Bool);
            match &observed.kind {
                ObservedSourceKind::Bool(actual) if *actual == *expected_value => {}
                _ => source_mismatch(case, path, expected, observed),
            }
        }
        Value::Char(expected_value) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(case, path, expected, observed, SourceTypeIdentity::Char);
            match &observed.kind {
                ObservedSourceKind::Char(actual) if *actual == *expected_value => {}
                _ => source_mismatch(case, path, expected, observed),
            }
        }
        Value::String(expected_value) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(case, path, expected, observed, SourceTypeIdentity::String);
            match &observed.kind {
                ObservedSourceKind::String(actual) if actual == expected_value => {}
                _ => source_mismatch(case, path, expected, observed),
            }
        }
        Value::Int(expected_value) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(case, path, expected, observed, SourceTypeIdentity::Int);
            match &observed.kind {
                ObservedSourceKind::Int(actual) if *actual == *expected_value => {}
                _ => source_mismatch(case, path, expected, observed),
            }
        }
        Value::Float(expected_value) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(case, path, expected, observed, SourceTypeIdentity::Float);
            match &observed.kind {
                ObservedSourceKind::Float(actual)
                    if actual.to_bits() == expected_value.to_bits() => {}
                _ => source_mismatch(case, path, expected, observed),
            }
        }
        Value::Decimal(_) => {
            let (expected_coefficient, expected_scale) = match identity_expectation
                .map(|value| &value.kind)
            {
                Some(SourceIdentityExpectationKind::Decimal { coefficient, scale }) => {
                    (coefficient, *scale)
                }
                _ => source_mismatch_message(
                    case,
                    path,
                    "MON Value::Decimal omits its declared coefficient and scale; add with_source_decimal for this path",
                ),
            };
            require_type(
                case,
                path,
                expected,
                observed,
                SourceTypeIdentity::Number(expected_scale),
            );
            let ObservedSourceKind::Number(actual) = &observed.kind else {
                source_mismatch(case, path, expected, observed);
            };
            if actual.scale().get() != expected_scale
                || actual.coefficient().to_string() != *expected_coefficient
            {
                source_mismatch_message(
                    case,
                    path,
                    &format!(
                        "expected decimal coefficient {expected_coefficient} at scale {expected_scale}, observed coefficient {} at scale {}",
                        actual.coefficient(),
                        actual.scale().get()
                    ),
                );
            }
        }
        Value::I8(value) => assert_fixed_signed(
            *value as i64,
            FixedScalar::I8,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::I16(value) => assert_fixed_signed(
            *value as i64,
            FixedScalar::I16,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::I32(value) => assert_fixed_signed(
            *value as i64,
            FixedScalar::I32,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::I64(value) => assert_fixed_signed(
            *value,
            FixedScalar::I64,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::U8(value) => assert_fixed_unsigned(
            *value as u64,
            FixedScalar::U8,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::U16(value) => assert_fixed_unsigned(
            *value as u64,
            FixedScalar::U16,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::U32(value) => assert_fixed_unsigned(
            *value as u64,
            FixedScalar::U32,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::U64(value) => assert_fixed_unsigned(
            *value,
            FixedScalar::U64,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::Byte(value) => assert_fixed_unsigned(
            *value as u64,
            FixedScalar::Byte,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::F16(value) => assert_fixed_float(
            *value,
            FixedScalar::F16,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::F32(value) => assert_fixed_float(
            *value,
            FixedScalar::F32,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::F64(value) => assert_fixed_float(
            *value,
            FixedScalar::F64,
            expected,
            observed,
            case,
            path,
            identity_expectation,
        ),
        Value::Collection(expected_values) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(
                case,
                path,
                expected,
                observed,
                SourceTypeIdentity::Collection,
            );
            let ObservedSourceKind::Collection(actual_values) = &observed.kind else {
                source_mismatch(case, path, expected, observed);
            };
            assert_eq!(
                expected_values.len(),
                actual_values.len(),
                "source typed value mismatch in case '{case}' at path '{path}': collection length"
            );
            for (index, (expected_value, actual_value)) in
                expected_values.iter().zip(actual_values).enumerate()
            {
                assert_source_matches(
                    expected_value,
                    actual_value,
                    case,
                    &format!("{path}[{index}]"),
                    identity_expectations,
                    optional_paths,
                );
            }
        }
        Value::Map(expected_entries) => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            require_type(case, path, expected, observed, SourceTypeIdentity::Map);
            let actual_entries = match &observed.kind {
                ObservedSourceKind::Map(entries) => entries,
                ObservedSourceKind::Unsupported(reason) => source_mismatch_message(
                    case,
                    path,
                    &format!("map observation unsupported: {reason}"),
                ),
                _ => source_mismatch(case, path, expected, observed),
            };
            assert_eq!(
                expected_entries.len(),
                actual_entries.len(),
                "source typed value mismatch in case '{case}' at path '{path}': map entry count"
            );
            for (index, ((expected_key, expected_value), (actual_key, actual_value))) in
                expected_entries.iter().zip(actual_entries).enumerate()
            {
                assert_source_matches(
                    expected_key,
                    actual_key,
                    case,
                    &format!("{path}[key:{index}]"),
                    identity_expectations,
                    optional_paths,
                );
                assert_source_matches(
                    expected_value,
                    actual_value,
                    case,
                    &format!("{path}[value:{index}]"),
                    identity_expectations,
                    optional_paths,
                );
            }
        }
        Value::Record(expected_fields) => {
            let expected_nominal = match identity_expectation.map(|value| &value.kind) {
                Some(SourceIdentityExpectationKind::Nominal(name)) => Some(name.as_str()),
                Some(SourceIdentityExpectationKind::Decimal { .. }) => source_mismatch_message(
                    case,
                    path,
                    "decimal identity expectation was attached to a non-decimal value",
                ),
                None => None,
            };
            let ObservedSourceKind::Record {
                nominal_name,
                fields: actual_fields,
            } = &observed.kind
            else {
                source_mismatch(case, path, expected, observed);
            };
            match (nominal_name.as_deref(), expected_nominal) {
                (Some(actual), Some(expected_name)) if actual == expected_name => {}
                (None, None) => {}
                _ => source_mismatch(case, path, expected, observed),
            }
            assert_ordered_fields_match(
                expected_fields,
                actual_fields,
                case,
                path,
                identity_expectations,
                optional_paths,
            );
        }
        Value::Choice {
            qualifier,
            variant,
            fields: expected_fields,
        } => {
            assert_identity_expectation_unused(
                identity_expectation,
                case,
                path,
                expected,
                observed,
            );
            let ObservedSourceKind::Choice {
                nominal_name,
                variant_name,
                fields: actual_fields,
            } = &observed.kind
            else {
                source_mismatch(case, path, expected, observed);
            };
            if observed.type_identity != SourceTypeIdentity::Choice(nominal_name.clone())
                || qualifier.as_deref() != Some(nominal_name.as_str())
                || variant != variant_name
            {
                source_mismatch(case, path, expected, observed);
            }
            assert_ordered_fields_match(
                expected_fields,
                actual_fields,
                case,
                path,
                identity_expectations,
                optional_paths,
            );
        }
        Value::Integer(_) => source_mismatch_message(
            case,
            path,
            "source Integer observation is unsupported because Moth has no arbitrary-precision Integer type",
        ),
    }
}

fn assert_ordered_fields_match(
    expected_fields: &[(String, Value)],
    actual_fields: &[ObservedSourceField],
    case: &str,
    path: &str,
    identity_expectations: &[SourceIdentityExpectation],
    optional_paths: &[String],
) {
    if expected_fields.len() != actual_fields.len() {
        source_mismatch_message(
            case,
            path,
            &format!(
                "field count differed: expected {:?}, observed {:?}",
                expected_fields
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
                actual_fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>()
            ),
        );
    }

    for (index, ((expected_name, expected_value), actual_field)) in
        expected_fields.iter().zip(actual_fields).enumerate()
    {
        let field_path = nested_path(path, &actual_field.name);
        if expected_name != &actual_field.name {
            source_mismatch_message(
                case,
                &field_path,
                &format!(
                    "record field order/name differed: expected {expected_name:?} at index {index}, observed {:?}",
                    actual_field.name
                ),
            );
        }
        assert_source_matches(
            expected_value,
            &actual_field.value,
            case,
            &field_path,
            identity_expectations,
            optional_paths,
        );
    }
}

fn assert_fixed_signed(
    expected_value: i64,
    scalar: FixedScalar,
    expected: &Value,
    observed: &ObservedSourceValue,
    case: &str,
    path: &str,
    identity_expectation: Option<&SourceIdentityExpectation>,
) {
    assert_identity_expectation_unused(identity_expectation, case, path, expected, observed);
    require_type(
        case,
        path,
        expected,
        observed,
        SourceTypeIdentity::FixedScalar(scalar),
    );
    match &observed.kind {
        ObservedSourceKind::FixedScalar(actual)
            if actual.scalar() == scalar && actual.as_i64() == Some(expected_value) => {}
        _ => source_mismatch(case, path, expected, observed),
    }
}

fn assert_fixed_unsigned(
    expected_value: u64,
    scalar: FixedScalar,
    expected: &Value,
    observed: &ObservedSourceValue,
    case: &str,
    path: &str,
    identity_expectation: Option<&SourceIdentityExpectation>,
) {
    assert_identity_expectation_unused(identity_expectation, case, path, expected, observed);
    require_type(
        case,
        path,
        expected,
        observed,
        SourceTypeIdentity::FixedScalar(scalar),
    );
    match &observed.kind {
        ObservedSourceKind::FixedScalar(actual)
            if actual.scalar() == scalar && actual.as_u64() == Some(expected_value) => {}
        _ => source_mismatch(case, path, expected, observed),
    }
}

fn assert_fixed_float(
    expected_value: f64,
    scalar: FixedScalar,
    expected: &Value,
    observed: &ObservedSourceValue,
    case: &str,
    path: &str,
    identity_expectation: Option<&SourceIdentityExpectation>,
) {
    assert_identity_expectation_unused(identity_expectation, case, path, expected, observed);
    require_type(
        case,
        path,
        expected,
        observed,
        SourceTypeIdentity::FixedScalar(scalar),
    );
    match &observed.kind {
        ObservedSourceKind::FixedScalar(actual)
            if actual.scalar() == scalar
                && actual
                    .as_f64()
                    .is_some_and(|value| value.to_bits() == expected_value.to_bits()) => {}
        _ => source_mismatch(case, path, expected, observed),
    }
}

fn assert_identity_expectation_unused(
    identity_expectation: Option<&SourceIdentityExpectation>,
    case: &str,
    path: &str,
    expected: &Value,
    observed: &ObservedSourceValue,
) {
    if identity_expectation.is_some() {
        source_mismatch_message(
            case,
            path,
            &format!(
                "unexpected extra source identity expectation for {:?}; observed {:?}",
                expected, observed.type_identity
            ),
        );
    }
}

fn require_type(
    case: &str,
    path: &str,
    expected: &Value,
    observed: &ObservedSourceValue,
    expected_identity: SourceTypeIdentity,
) {
    if observed.type_identity != expected_identity {
        source_mismatch_message(
            case,
            path,
            &format!(
                "declared source identity differed: expected {expected_identity:?}, observed {:?}; expected value {expected:?}",
                observed.type_identity
            ),
        );
    }
}

fn source_mismatch(case: &str, path: &str, expected: &Value, observed: &ObservedSourceValue) -> ! {
    source_mismatch_message(
        case,
        path,
        &format!("expected {expected:?}, observed {observed:?}"),
    )
}

fn source_mismatch_message(case: &str, path: &str, detail: &str) -> ! {
    panic!("source typed value mismatch in case '{case}' at path '{path}': {detail}")
}

fn observe_stored_value(
    store: &ConstValueStore,
    id: ConstValueId,
    context: &ObservationContext<'_>,
) -> ObservedSourceValue {
    let metadata = store
        .metadata(id)
        .expect("module constant row and child references must name stored values");
    let type_identity = source_type_identity(metadata.type_id, context);
    let payload = store
        .payload(id)
        .expect("module constant row and child references must name stored values");
    let kind = match payload {
        ConstValuePayload::Int(value) => ObservedSourceKind::Int(*value),
        ConstValuePayload::Float(value) => ObservedSourceKind::Float(*value),
        ConstValuePayload::Number(value) => ObservedSourceKind::Number(value.clone()),
        ConstValuePayload::FixedScalar(value) => ObservedSourceKind::FixedScalar(*value),
        ConstValuePayload::Bool(value) => ObservedSourceKind::Bool(*value),
        ConstValuePayload::Char(value) => ObservedSourceKind::Char(*value),
        ConstValuePayload::String(value) => observe_const_string(value, context),
        ConstValuePayload::Collection(values) => ObservedSourceKind::Collection(
            values
                .iter()
                .map(|value| observe_stored_value(store, *value, context))
                .collect(),
        ),
        ConstValuePayload::Record(fields) => ObservedSourceKind::Record {
            nominal_name: match &type_identity {
                SourceTypeIdentity::Struct(name) => Some(name.clone()),
                _ => None,
            },
            fields: fields
                .iter()
                .map(|field| ObservedSourceField {
                    name: path_component_text(field.name, context),
                    value: observe_stored_value(store, field.value, context),
                })
                .collect(),
        },
        ConstValuePayload::Choice {
            nominal_path,
            tag,
            fields,
        } => ObservedSourceKind::Choice {
            nominal_name: path_component_text(*nominal_path, context),
            variant_name: choice_variant_name(*nominal_path, *tag, context),
            fields: fields
                .iter()
                .map(|field| ObservedSourceField {
                    name: path_component_text(field.name, context),
                    value: observe_stored_value(store, field.value, context),
                })
                .collect(),
        },
        ConstValuePayload::Range { .. } => ObservedSourceKind::Unsupported(
            "folded range values are outside MON Value parity".into(),
        ),
        ConstValuePayload::Coerced(child) => {
            let mut value = observe_stored_value(store, *child, context);
            value.type_identity = type_identity.clone();
            return value;
        }
        ConstValuePayload::OptionSome(child) => {
            ObservedSourceKind::OptionSome(Box::new(observe_stored_value(store, *child, context)))
        }
        ConstValuePayload::OptionNone => ObservedSourceKind::OptionNone,
        ConstValuePayload::Template { folded, .. } => match folded {
            Some(value) => observe_const_string(value, context),
            None => ObservedSourceKind::Unsupported(
                "unfolded template has no ordinary string observation".into(),
            ),
        },
    };

    ObservedSourceValue {
        type_identity,
        kind,
    }
}

fn observe_const_string(
    value: &ConstStringValue,
    context: &ObservationContext<'_>,
) -> ObservedSourceKind {
    match value {
        ConstStringValue::Text(text) => ObservedSourceKind::String(resolve_string(*text, context)),
        ConstStringValue::Pieces(pieces) => observe_string_pieces(pieces, context),
    }
}

fn observe_string_pieces(
    pieces: &[ConstStringPiece],
    context: &ObservationContext<'_>,
) -> ObservedSourceKind {
    let mut text = String::new();
    for piece in pieces {
        match piece {
            ConstStringPiece::Text(part) => text.push_str(&resolve_string(*part, context)),
            ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                return ObservedSourceKind::Unsupported(
                    "structural strings with unresolved resource pieces are not plain MON strings"
                        .into(),
                );
            }
        }
    }
    ObservedSourceKind::String(text)
}

fn observe_expression(
    expression: &Expression,
    context: &ObservationContext<'_>,
) -> ObservedSourceValue {
    let type_identity = source_type_identity(expression.type_id, context);
    let kind = match &expression.kind {
        ExpressionKind::Int(value) => ObservedSourceKind::Int(*value),
        ExpressionKind::Float(value) => ObservedSourceKind::Float(*value),
        ExpressionKind::Number(value) => ObservedSourceKind::Number(value.clone()),
        ExpressionKind::FixedScalar(value) => ObservedSourceKind::FixedScalar(*value),
        ExpressionKind::Bool(value) => ObservedSourceKind::Bool(*value),
        ExpressionKind::Char(value) => ObservedSourceKind::Char(*value),
        ExpressionKind::StringSlice(value) => {
            ObservedSourceKind::String(resolve_string(*value, context))
        }
        ExpressionKind::StructuralString { pieces } => observe_string_pieces(pieces, context),
        ExpressionKind::Reference(path) => {
            let start_body = function_body_by_name(
                context.ast,
                context.path_fork,
                context.string_table,
                IMPLICIT_START_FUNC_NAME,
            );
            let declaration = start_body.iter().find_map(|node| match &node.kind {
                NodeKind::VariableDeclaration(declaration) if declaration.id == *path => {
                    Some(declaration)
                }
                _ => None,
            });
            let Some(declaration) = declaration else {
                return ObservedSourceValue {
                    type_identity,
                    kind: ObservedSourceKind::Unsupported(format!(
                        "source reference {path:?} is not a start-body local"
                    )),
                };
            };
            let mut observed = observe_expression(&declaration.value, context);
            observed.type_identity = type_identity;
            return observed;
        }
        ExpressionKind::Collection(values) => ObservedSourceKind::Collection(
            values
                .iter()
                .map(|value| observe_expression(value, context))
                .collect(),
        ),
        ExpressionKind::MapLiteral(entries) => ObservedSourceKind::Map(
            entries
                .iter()
                .map(|entry| {
                    (
                        observe_expression(&entry.key, context),
                        observe_expression(&entry.value, context),
                    )
                })
                .collect(),
        ),
        ExpressionKind::StructInstance(fields) => ObservedSourceKind::Record {
            nominal_name: match &type_identity {
                SourceTypeIdentity::Struct(name) => Some(name.clone()),
                _ => None,
            },
            fields: observe_declarations(fields, context),
        },
        ExpressionKind::AnonymousConstRecord { fields } => ObservedSourceKind::Record {
            nominal_name: None,
            fields: observe_declarations(fields, context),
        },
        ExpressionKind::ChoiceConstruct {
            nominal_path,
            tag,
            fields,
        } => ObservedSourceKind::Choice {
            nominal_name: path_component_text(*nominal_path, context),
            variant_name: choice_variant_name(*nominal_path, *tag, context),
            fields: observe_declarations(fields, context),
        },
        ExpressionKind::OptionNone => ObservedSourceKind::OptionNone,
        ExpressionKind::Coerced { value, .. }
            if matches!(type_identity, SourceTypeIdentity::Option) =>
        {
            ObservedSourceKind::OptionSome(Box::new(observe_expression(value, context)))
        }
        ExpressionKind::Coerced { value, .. } => {
            let mut observed = observe_expression(value, context);
            observed.type_identity = type_identity.clone();
            return observed;
        }
        ExpressionKind::Range(start, end) => ObservedSourceKind::Unsupported(format!(
            "range from {:?} to {:?} is not a MON Value carrier",
            start.kind, end.kind
        )),
        other => ObservedSourceKind::Unsupported(format!(
            "source expression kind {:?} has no parity observation adapter",
            other
        )),
    };

    ObservedSourceValue {
        type_identity,
        kind,
    }
}

fn observe_declarations(
    declarations: &[crate::compiler_frontend::ast::ast_nodes::Declaration],
    context: &ObservationContext<'_>,
) -> Vec<ObservedSourceField> {
    declarations
        .iter()
        .map(|declaration| ObservedSourceField {
            name: path_component_text(declaration.id, context),
            value: observe_expression(&declaration.value, context),
        })
        .collect()
}

fn source_type_identity(type_id: TypeId, context: &ObservationContext<'_>) -> SourceTypeIdentity {
    let Some(definition) = context.ast.type_environment.get(type_id) else {
        return SourceTypeIdentity::Other(format!("unknown TypeId {type_id:?}"));
    };

    match definition {
        TypeDefinition::Builtin(builtin) => match builtin.key {
            BuiltinTypeKey::Bool => SourceTypeIdentity::Bool,
            BuiltinTypeKey::Int => SourceTypeIdentity::Int,
            BuiltinTypeKey::Float => SourceTypeIdentity::Float,
            BuiltinTypeKey::String => SourceTypeIdentity::String,
            BuiltinTypeKey::Char => SourceTypeIdentity::Char,
            BuiltinTypeKey::None => SourceTypeIdentity::None,
            BuiltinTypeKey::Number(scale) => SourceTypeIdentity::Number(scale.get()),
            BuiltinTypeKey::FixedScalar(scalar) => SourceTypeIdentity::FixedScalar(scalar),
            other => SourceTypeIdentity::Other(format!("builtin {other:?}")),
        },
        TypeDefinition::Struct(definition) => {
            SourceTypeIdentity::Struct(path_component_text(definition.path, context))
        }
        TypeDefinition::Choice(definition) => {
            SourceTypeIdentity::Choice(path_component_text(definition.path, context))
        }
        TypeDefinition::AnonymousConstRecordMarker => SourceTypeIdentity::AnonymousRecord,
        TypeDefinition::Constructed(definition) => match &definition.constructor {
            TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { .. }) => {
                SourceTypeIdentity::Collection
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => SourceTypeIdentity::Map,
            TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => SourceTypeIdentity::Option,
            other => SourceTypeIdentity::Other(format!("constructed type {other:?}")),
        },
        TypeDefinition::GenericInstance(definition) => {
            let Some(base_type_id) = context
                .ast
                .type_environment
                .type_id_for_nominal_id(definition.base)
            else {
                return SourceTypeIdentity::Other("generic nominal base was missing".into());
            };
            match context.ast.type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(base)) => {
                    SourceTypeIdentity::Struct(path_component_text(base.path, context))
                }
                Some(TypeDefinition::Choice(base)) => {
                    SourceTypeIdentity::Choice(path_component_text(base.path, context))
                }
                _ => SourceTypeIdentity::Other("generic nominal base was not struct/choice".into()),
            }
        }
        other => SourceTypeIdentity::Other(format!("type definition {other:?}")),
    }
}

fn choice_variant_name(
    nominal_path: PathId,
    tag: usize,
    context: &ObservationContext<'_>,
) -> String {
    let nominal_id = context
        .ast
        .type_environment
        .nominal_id_for_path(&nominal_path)
        .expect("choice payload path must identify its registered nominal type");
    let definition = context
        .ast
        .type_environment
        .choice_definition(nominal_id)
        .expect("choice payload type must have a choice definition");
    let variant = definition
        .variants
        .iter()
        .find(|variant| variant.tag == tag)
        .expect("choice payload tag must identify a declared variant");
    resolve_string(variant.name, context)
}

fn start_body_expression(node: &AstNode) -> Option<&Expression> {
    match &node.kind {
        NodeKind::ExpressionStatement(expression) => Some(expression),
        NodeKind::VariableDeclaration(declaration) => Some(&declaration.value),
        _ => None,
    }
}

fn call_arguments_for_function<'a>(
    expression: &'a Expression,
    expected_function_name: &str,
    context: &ObservationContext<'_>,
) -> Option<&'a [CallArgument]> {
    let (name, arguments) = match &expression.kind {
        ExpressionKind::FunctionCall { name, args, .. } => (*name, args),
        _ => return None,
    };
    (path_component_text(name, context) == expected_function_name).then_some(arguments)
}

fn path_component_text(path: PathId, context: &ObservationContext<'_>) -> String {
    let component = context
        .path_fork
        .try_component(path)
        .expect("AST path must belong to its parse path fork");
    resolve_string(component, context)
}

fn resolve_string(string: StringId, context: &ObservationContext<'_>) -> String {
    context.string_table.resolve(string).to_owned()
}

fn nested_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}.{child}")
    }
}

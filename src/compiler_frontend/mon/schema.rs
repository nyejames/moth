//! Immutable MON schema preparation and default validation.
//!
//! This module owns the compiler-independent schema boundary. It converts the caller's
//! owned schema description into a validated representation that the reader can inspect,
//! checks every concrete member transitively, and stores completed defaults without any
//! compiler identity or source-parser state.

use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::numeric_text::parse::parse_numeric_literal;
use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;

use super::{
    BudgetState, Field, Limits, MonError, MonErrorCode, PathSegment, PreparedSchema, Schema,
    SchemaType, Value,
};

/// A schema type after eligibility, names, scales and defaults have been checked.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PreparedType {
    None,
    Bool,
    Char,
    String,
    Int,
    Float,
    Integer,
    Decimal {
        scale: u8,
    },
    Optional(Box<PreparedType>),
    Record {
        fields: Vec<PreparedField>,
    },
    Struct {
        name: String,
        fields: Vec<PreparedField>,
    },
    Collection {
        element: Box<PreparedType>,
    },
    Map {
        key: Box<PreparedType>,
        value: Box<PreparedType>,
    },
    Choice {
        name: String,
        variants: Vec<PreparedVariant>,
    },
}

/// A prepared closed-schema field and its recursively completed default.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PreparedField {
    pub(super) name: String,
    pub(super) ty: PreparedType,
    pub(super) default: Option<Value>,
}

/// A prepared nominal choice variant. Payload fields are always required.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PreparedVariant {
    pub(super) name: String,
    pub(super) fields: Vec<PreparedField>,
}

/// Prepare a schema once so later reader calls only inspect immutable validated data.
///
/// A value schema may have any root type. Document framing is the reader's concern and
/// rejects a non-record root only for the document entry point; nested-value callers use
/// the same prepared representation without that document-only restriction.
pub(crate) fn prepare_schema(schema: Schema) -> Result<PreparedSchema, MonError> {
    let Schema { root, limits } = schema;
    let mut path = Vec::new();
    let root = {
        let mut budget = BudgetState::new(&limits);
        prepare_type(root, &mut budget, &mut path, 0)?
    };

    Ok(PreparedSchema { root, limits })
}

/// Validate an owned value against a prepared type and fill only the exact record fields
/// omitted by that record's own defaults. The reader may use this for programmatically
/// supplied nested values after parsing; defaults are already complete in the prepared
/// fields, and a supplied `None` is never replaced.
pub(super) fn validate_and_complete_value(
    value: Value,
    ty: &PreparedType,
    limits: &Limits,
) -> Result<Value, MonError> {
    let mut path = Vec::new();
    let mut budget = BudgetState::new(limits);
    complete_value(value, ty, &mut budget, &mut path, 0)
}

fn prepare_type(
    schema_type: SchemaType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<PreparedType, MonError> {
    check_depth(budget, depth, path)?;
    charge_nodes(budget, 1, path)?;

    match schema_type {
        SchemaType::None => Ok(PreparedType::None),
        SchemaType::Bool => Ok(PreparedType::Bool),
        SchemaType::Char => Ok(PreparedType::Char),
        SchemaType::String => Ok(PreparedType::String),
        SchemaType::Int => Ok(PreparedType::Int),
        SchemaType::Float => Ok(PreparedType::Float),
        SchemaType::Integer => Ok(PreparedType::Integer),

        SchemaType::Decimal { scale } => {
            if scale > 18 {
                return Err(schema_error(
                    MonErrorCode::NumericScale,
                    path,
                    format!("Decimal scale {scale} is outside the supported range 0..=18"),
                ));
            }
            Ok(PreparedType::Decimal { scale })
        }

        SchemaType::Optional(inner) => {
            let inner = prepare_type(*inner, budget, path, depth + 1)?;
            if accepts_none(&inner) {
                return Err(schema_error(
                    MonErrorCode::InvalidSchema,
                    path,
                    "optional schema cannot contain another nullable schema",
                ));
            }
            Ok(PreparedType::Optional(Box::new(inner)))
        }

        SchemaType::Record { fields } => Ok(PreparedType::Record {
            fields: prepare_fields(fields, budget, path, true, depth)?,
        }),

        SchemaType::Struct { name, fields } => {
            charge_decoded_bytes(budget, name.len(), path)?;
            validate_identifier(&name, path, "struct name")?;
            Ok(PreparedType::Struct {
                name,
                fields: prepare_fields(fields, budget, path, true, depth)?,
            })
        }

        SchemaType::Collection { element } => Ok(PreparedType::Collection {
            element: Box::new(prepare_type(*element, budget, path, depth + 1)?),
        }),

        SchemaType::Map { key, value } => {
            let key = prepare_type(*key, budget, path, depth + 1)?;
            if !is_supported_map_key(&key) {
                return Err(schema_error(
                    MonErrorCode::InvalidMapKey,
                    path,
                    "map keys must be String, Int, Bool or Char",
                ));
            }

            let value = prepare_type(*value, budget, path, depth + 1)?;
            Ok(PreparedType::Map {
                key: Box::new(key),
                value: Box::new(value),
            })
        }

        SchemaType::Choice { name, variants } => {
            charge_decoded_bytes(budget, name.len(), path)?;
            validate_identifier(&name, path, "choice name")?;
            let mut prepared_variants = Vec::new();

            for variant in variants {
                charge_decoded_bytes(budget, variant.name.len(), path)?;
                charge_nodes(budget, 1, path)?;
                if prepared_variants
                    .iter()
                    .any(|known: &PreparedVariant| known.name == variant.name)
                {
                    let mut variant_path = path.clone();
                    variant_path.push(PathSegment::Variant(variant.name.clone()));
                    return Err(schema_error(
                        MonErrorCode::InvalidSchema,
                        &variant_path,
                        format!(
                            "duplicate choice variant '{}'; names must be unique",
                            variant.name
                        ),
                    ));
                }
                path.push(PathSegment::Variant(variant.name.clone()));
                validate_identifier(&variant.name, path, "choice variant name")?;
                let prepared_fields = prepare_fields(variant.fields, budget, path, false, depth)?;
                path.pop();
                prepared_variants.push(PreparedVariant {
                    name: variant.name,
                    fields: prepared_fields,
                });
            }

            Ok(PreparedType::Choice {
                name,
                variants: prepared_variants,
            })
        }

        SchemaType::Unsupported { name } => {
            charge_decoded_bytes(budget, name.len(), path)?;
            validate_identifier(&name, path, "unsupported schema name")?;
            Err(schema_error(
                MonErrorCode::UnsupportedSchema,
                path,
                format!("schema member '{name}' is unsupported by MON"),
            ))
        }
    }
}

fn prepare_fields(
    fields: Vec<Field>,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    allow_defaults: bool,
    depth: usize,
) -> Result<Vec<PreparedField>, MonError> {
    let mut prepared_fields = Vec::new();

    for field in fields {
        charge_nodes(budget, 1, path)?;
        validate_identifier(&field.name, path, "field name")?;
        if prepared_fields
            .iter()
            .any(|known: &PreparedField| known.name == field.name)
        {
            charge_decoded_bytes(budget, field.name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(field.name.clone()));
            return Err(schema_error(
                MonErrorCode::DuplicateField,
                &field_path,
                format!("duplicate field '{}'; names must be unique", field.name),
            ));
        }

        if !allow_defaults && field.default.is_some() {
            charge_decoded_bytes(budget, field.name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(field.name.clone()));
            return Err(schema_error(
                MonErrorCode::InvalidSchema,
                &field_path,
                "choice payload fields cannot have defaults",
            ));
        }

        prepared_fields.push(prepare_field(
            field,
            budget,
            path,
            allow_defaults,
            depth + 1,
        )?);
    }

    Ok(prepared_fields)
}

fn prepare_field(
    field: Field,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    allow_defaults: bool,
    depth: usize,
) -> Result<PreparedField, MonError> {
    let Field { name, ty, default } = field;

    charge_decoded_bytes(budget, name.len(), path)?;
    path.push(PathSegment::Field(name.clone()));
    let ty = prepare_type(ty, budget, path, depth)?;
    let default = match default {
        Some(value) => {
            if !allow_defaults {
                path.pop();
                return Err(schema_error(
                    MonErrorCode::InvalidSchema,
                    path,
                    "choice payload fields cannot have defaults",
                ));
            }
            Some(complete_value(value, &ty, budget, path, depth)?)
        }
        None => None,
    };
    path.pop();

    Ok(PreparedField { name, ty, default })
}

fn validate_identifier(name: &str, path: &[PathSegment], kind: &str) -> Result<(), MonError> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(schema_error(
            MonErrorCode::InvalidIdentifier,
            path,
            format!("{kind} must not be empty"),
        ));
    };

    if first != '_' && !first.is_alphabetic() {
        return Err(schema_error(
            MonErrorCode::InvalidIdentifier,
            path,
            format!("invalid {kind}: expected an alphabetic or '_' start"),
        ));
    }

    if characters.any(|character| character != '_' && !character.is_alphanumeric()) {
        return Err(schema_error(
            MonErrorCode::InvalidIdentifier,
            path,
            format!("invalid {kind}: continuation must be alphanumeric or '_'"),
        ));
    }

    Ok(())
}

fn is_supported_map_key(ty: &PreparedType) -> bool {
    matches!(
        ty,
        PreparedType::String | PreparedType::Int | PreparedType::Bool | PreparedType::Char
    )
}

fn accepts_none(ty: &PreparedType) -> bool {
    matches!(ty, PreparedType::None | PreparedType::Optional(_))
}

fn with_budget_path(mut error: MonError, path: &[PathSegment]) -> MonError {
    error.path = path.to_owned();
    error
}

fn check_depth(
    budget: &BudgetState<'_>,
    depth: usize,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .check_depth(depth, None)
        .map_err(|error| with_budget_path(error, path))
}

fn charge_nodes(
    budget: &mut BudgetState<'_>,
    count: usize,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .charge_nodes(count, None)
        .map_err(|error| with_budget_path(error, path))
}

fn charge_decoded_bytes(
    budget: &mut BudgetState<'_>,
    count: usize,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .charge_decoded_bytes(count, None)
        .map_err(|error| with_budget_path(error, path))
}

fn charge_default(budget: &mut BudgetState<'_>, path: &[PathSegment]) -> Result<(), MonError> {
    budget
        .charge_default(None)
        .map_err(|error| with_budget_path(error, path))
}

fn complete_value(
    value: Value,
    ty: &PreparedType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Value, MonError> {
    check_depth(budget, depth, path)?;
    if let PreparedType::Optional(inner) = ty {
        if matches!(&value, Value::None) {
            charge_nodes(budget, 1, path)?;
            return Ok(Value::None);
        }
        return complete_value(value, inner, budget, path, depth);
    }
    charge_nodes(budget, 1, path)?;

    match (value, ty) {
        (Value::None, PreparedType::None) => Ok(Value::None),
        (Value::None, _) => Err(invalid_default(
            path,
            "none is not accepted by this schema type",
        )),

        (Value::Bool(value), PreparedType::Bool) => Ok(Value::Bool(value)),
        (Value::Int(value), PreparedType::Int) => Ok(Value::Int(value)),
        (Value::Char(value), PreparedType::Char) => {
            charge_decoded_bytes(budget, value.len_utf8(), path)?;
            Ok(Value::Char(value))
        }
        (Value::String(value), PreparedType::String) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            Ok(Value::String(value))
        }
        (Value::Float(value), PreparedType::Float) => {
            if value.is_finite() {
                Ok(Value::Float(value))
            } else {
                Err(invalid_default(path, "Float defaults must be finite"))
            }
        }

        (Value::Integer(value), PreparedType::Integer) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            validate_integer_text(&value, budget.limits, path)?;
            Ok(Value::Integer(value))
        }
        (Value::Decimal(value), PreparedType::Decimal { scale }) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            validate_decimal_text(&value, *scale, budget.limits, path)?;
            Ok(Value::Decimal(value))
        }

        (Value::Record(values), PreparedType::Record { fields })
        | (Value::Record(values), PreparedType::Struct { fields, .. }) => {
            let values = complete_record(values, fields, budget, path, depth)?;
            Ok(Value::Record(values))
        }

        (Value::Collection(values), PreparedType::Collection { element }) => {
            let mut completed = Vec::new();
            for (index, value) in values.into_iter().enumerate() {
                path.push(PathSegment::Index(index));
                let value = complete_value(value, element, budget, path, depth + 1)?;
                path.pop();
                completed.push(value);
            }
            Ok(Value::Collection(completed))
        }

        (Value::Map(entries), PreparedType::Map { key, value }) => {
            let mut completed = Vec::new();
            for (index, (key_value, value_value)) in entries.into_iter().enumerate() {
                path.push(PathSegment::Index(index));
                let key_value = complete_value(key_value, key, budget, path, depth + 1)?;
                if completed
                    .iter()
                    .any(|(known_key, _)| known_key == &key_value)
                {
                    return Err(invalid_default(path, "duplicate map key in default value"));
                }
                let value_value = complete_value(value_value, value, budget, path, depth + 1)?;
                path.pop();
                completed.push((key_value, value_value));
            }
            Ok(Value::Map(completed))
        }

        (
            Value::Choice {
                qualifier,
                variant,
                fields: values,
            },
            PreparedType::Choice { name, variants },
        ) => {
            if let Some(qualifier) = qualifier.as_deref() {
                if !is_identifier(qualifier) {
                    return Err(invalid_default(
                        path,
                        "choice qualifier is not a valid identifier",
                    ));
                }
                if qualifier != name {
                    charge_decoded_bytes(budget, qualifier.len(), path)?;
                    return Err(schema_error(
                        MonErrorCode::QualifierMismatch,
                        path,
                        format!("choice qualifier '{qualifier}' does not match '{name}'"),
                    ));
                }
            }
            let qualifier = match qualifier {
                Some(value) => {
                    charge_decoded_bytes(budget, value.len(), path)?;
                    Some(value)
                }
                None => None,
            };
            charge_decoded_bytes(budget, variant.len(), path)?;
            let Some(expected_variant) =
                variants.iter().find(|candidate| candidate.name == variant)
            else {
                return Err(schema_error(
                    MonErrorCode::UnknownVariant,
                    path,
                    format!("unknown choice variant '{variant}'"),
                ));
            };

            path.push(PathSegment::Variant(variant.clone()));
            let fields = complete_record(values, &expected_variant.fields, budget, path, depth)?;
            path.pop();

            Ok(Value::Choice {
                qualifier,
                variant,
                fields,
            })
        }

        (_, _) => Err(invalid_default(
            path,
            "value does not match its receiving schema type",
        )),
    }
}

fn complete_record(
    values: Vec<(String, Value)>,
    fields: &[PreparedField],
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Vec<(String, Value)>, MonError> {
    charge_nodes(budget, fields.len(), path)?;
    let mut supplied: Vec<Option<Value>> = (0..fields.len()).map(|_| None).collect();

    for (name, value) in values {
        if !is_identifier(&name) {
            charge_decoded_bytes(budget, name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(name));
            return Err(invalid_default(
                &field_path,
                "record field name is not a valid identifier",
            ));
        }

        let Some(index) = fields.iter().position(|field| field.name == name) else {
            charge_decoded_bytes(budget, name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(name.clone()));
            return Err(schema_error(
                MonErrorCode::UnknownField,
                &field_path,
                format!("unknown field '{name}' in default value"),
            ));
        };

        if supplied[index].is_some() {
            charge_decoded_bytes(budget, name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(name.clone()));
            return Err(schema_error(
                MonErrorCode::DuplicateField,
                &field_path,
                format!("duplicate field '{name}' in default value"),
            ));
        }
        supplied[index] = Some(value);
    }

    let mut completed = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        charge_decoded_bytes(budget, field.name.len(), path)?;
        path.push(PathSegment::Field(field.name.clone()));
        let value = match supplied[index].take() {
            Some(value) => complete_value(value, &field.ty, budget, path, depth + 1)?,
            None => match &field.default {
                Some(default) => {
                    charge_default(budget, path)?;
                    clone_default(default, budget, path, depth + 1)?
                }
                None => {
                    return Err(invalid_default(
                        path,
                        format!("required field '{}' is missing", field.name),
                    ));
                }
            },
        };
        path.pop();
        completed.push((field.name.clone(), value));
    }

    Ok(completed)
}

fn clone_default(
    value: &Value,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Value, MonError> {
    check_depth(budget, depth, path)?;
    charge_nodes(budget, 1, path)?;

    match value {
        Value::None => Ok(Value::None),
        Value::Bool(value) => Ok(Value::Bool(*value)),
        Value::Char(value) => {
            charge_decoded_bytes(budget, value.len_utf8(), path)?;
            Ok(Value::Char(*value))
        }
        Value::String(value) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            Ok(Value::String(value.clone()))
        }
        Value::Int(value) => Ok(Value::Int(*value)),
        Value::Float(value) => Ok(Value::Float(*value)),
        Value::Integer(value) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            Ok(Value::Integer(value.clone()))
        }
        Value::Decimal(value) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            Ok(Value::Decimal(value.clone()))
        }
        Value::Record(fields) => {
            let mut output = Vec::new();
            for (name, value) in fields {
                charge_decoded_bytes(budget, name.len(), path)?;
                let path_name = name.clone();
                path.push(PathSegment::Field(path_name));
                let copied = clone_default(value, budget, path, depth + 1)?;
                path.pop();
                charge_decoded_bytes(budget, name.len(), path)?;
                output.push((name.clone(), copied));
            }
            Ok(Value::Record(output))
        }
        Value::Collection(values) => {
            let mut output = Vec::new();
            for (index, value) in values.iter().enumerate() {
                path.push(PathSegment::Index(index));
                let copied = clone_default(value, budget, path, depth + 1)?;
                path.pop();
                output.push(copied);
            }
            Ok(Value::Collection(output))
        }
        Value::Map(entries) => {
            let mut output = Vec::new();
            for (index, (key, value)) in entries.iter().enumerate() {
                path.push(PathSegment::Index(index));
                let copied_key = clone_default(key, budget, path, depth + 1)?;
                let copied_value = clone_default(value, budget, path, depth + 1)?;
                path.pop();
                output.push((copied_key, copied_value));
            }
            Ok(Value::Map(output))
        }
        Value::Choice {
            qualifier,
            variant,
            fields,
        } => {
            let qualifier = match qualifier {
                Some(value) => {
                    charge_decoded_bytes(budget, value.len(), path)?;
                    Some(value.clone())
                }
                None => None,
            };
            charge_decoded_bytes(budget, variant.len(), path)?;
            let variant_name = variant.clone();
            charge_decoded_bytes(budget, variant.len(), path)?;
            path.push(PathSegment::Variant(variant.clone()));
            let mut copied_fields = Vec::new();
            for (name, value) in fields {
                charge_decoded_bytes(budget, name.len(), path)?;
                let path_name = name.clone();
                path.push(PathSegment::Field(path_name));
                let copied = clone_default(value, budget, path, depth + 1)?;
                path.pop();
                charge_decoded_bytes(budget, name.len(), path)?;
                copied_fields.push((name.clone(), copied));
            }
            path.pop();
            Ok(Value::Choice {
                qualifier,
                variant: variant_name,
                fields: copied_fields,
            })
        }
    }
}

fn validate_integer_text(
    text: &str,
    limits: &Limits,
    path: &[PathSegment],
) -> Result<(), MonError> {
    check_numeric_text_budget(text, limits, path)?;
    let unsigned = unsigned_numeric_text(text);
    let parsed =
        parse_numeric_literal(unsigned).map_err(|reason| numeric_default_error(path, reason))?;
    check_numeric_budget(parsed.digit_count as usize, limits, path)?;

    if parsed.kind != NumericLiteralKind::WholeNumber {
        return Err(invalid_default(
            path,
            "Integer defaults require whole-number spelling",
        ));
    }

    Ok(())
}

fn validate_decimal_text(
    text: &str,
    scale: u8,
    limits: &Limits,
    path: &[PathSegment],
) -> Result<(), MonError> {
    check_numeric_text_budget(text, limits, path)?;
    let unsigned = unsigned_numeric_text(text);
    let parsed =
        parse_numeric_literal(unsigned).map_err(|reason| numeric_default_error(path, reason))?;
    check_numeric_budget(parsed.digit_count as usize, limits, path)?;

    let Some(exponent_separator) = parsed.normalized_text.find('e') else {
        return check_decimal_scale(&parsed.normalized_text, 0, false, scale, path);
    };
    let coefficient = &parsed.normalized_text[..exponent_separator];
    let exponent = &parsed.normalized_text[(exponent_separator + 1)..];
    let negative_exponent = exponent.starts_with('-');
    let exponent_digits = exponent
        .strip_prefix('+')
        .or_else(|| exponent.strip_prefix('-'))
        .unwrap_or(exponent);
    let exponent_magnitude = saturating_decimal_usize(exponent_digits);

    check_decimal_scale(
        coefficient,
        exponent_magnitude,
        negative_exponent,
        scale,
        path,
    )
}

fn check_decimal_scale(
    coefficient: &str,
    exponent_magnitude: usize,
    negative_exponent: bool,
    scale: u8,
    path: &[PathSegment],
) -> Result<(), MonError> {
    let (integer_part, fractional_part) = match coefficient.split_once('.') {
        Some((integer_part, fractional_part)) => (integer_part, fractional_part),
        None => (coefficient, ""),
    };
    let fractional_digits = fractional_part.len();
    let all_zero = integer_part
        .bytes()
        .chain(fractional_part.bytes())
        .all(|character| character == b'0');
    if all_zero {
        return Ok(());
    }

    let trailing_zeroes = integer_part
        .bytes()
        .chain(fractional_part.bytes())
        .rev()
        .take_while(|character| *character == b'0')
        .count();
    let untrimmed_scale = if negative_exponent {
        fractional_digits.saturating_add(exponent_magnitude)
    } else {
        fractional_digits.saturating_sub(exponent_magnitude)
    };
    let effective_scale = untrimmed_scale.saturating_sub(trailing_zeroes);

    if effective_scale > scale as usize {
        return Err(schema_error(
            MonErrorCode::NumericScale,
            path,
            format!(
                "Decimal value has effective scale {effective_scale}, above declared scale {scale}"
            ),
        ));
    }

    Ok(())
}

fn unsigned_numeric_text(text: &str) -> &str {
    text.strip_prefix('-').unwrap_or(text)
}

fn saturating_decimal_usize(text: &str) -> usize {
    let mut value = 0usize;
    for character in text.bytes() {
        let digit = usize::from(character.saturating_sub(b'0'));
        value = value.saturating_mul(10).saturating_add(digit);
    }
    value
}

fn check_numeric_text_budget(
    text: &str,
    limits: &Limits,
    path: &[PathSegment],
) -> Result<(), MonError> {
    let digit_count = text
        .bytes()
        .filter(|character| character.is_ascii_digit())
        .count();
    check_numeric_budget(digit_count, limits, path)
}

fn check_numeric_budget(
    digit_count: usize,
    limits: &Limits,
    path: &[PathSegment],
) -> Result<(), MonError> {
    if digit_count > limits.max_numeric_digits {
        return Err(schema_error(
            MonErrorCode::NumericBudget,
            path,
            format!(
                "numeric default has {} digits, above limit {}",
                digit_count, limits.max_numeric_digits
            ),
        ));
    }
    Ok(())
}

fn numeric_default_error(path: &[PathSegment], reason: NumberLiteralErrorReason) -> MonError {
    invalid_default(path, format!("invalid numeric default: {reason:?}"))
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn invalid_default(path: &[PathSegment], detail: impl Into<String>) -> MonError {
    schema_error(MonErrorCode::InvalidDefault, path, detail)
}

fn schema_error(code: MonErrorCode, path: &[PathSegment], detail: impl Into<String>) -> MonError {
    MonError::new(code, None, path, detail)
}

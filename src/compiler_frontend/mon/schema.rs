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
    BudgetState, Field, Limits, MapKeyIndex, MonError, MonErrorCode, PathSegment, PreparedSchema,
    Schema, SchemaType, Value, Variant,
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
    if limits.max_depth > Limits::MAX_SAFE_DEPTH {
        let requested = limits.max_depth;
        drop_schema_type_tree(root);
        return Err(MonError::new(
            MonErrorCode::DepthBudget,
            None,
            &[],
            format!(
                "MON max_depth {requested} exceeds implementation-safe ceiling {}",
                Limits::MAX_SAFE_DEPTH
            ),
        ));
    }
    let mut path = Vec::new();
    let root = {
        let mut budget = BudgetState::new(&limits);
        prepare_type(root, &mut budget, &mut path, 0)?
    };

    Ok(PreparedSchema { root, limits })
}

/// Validate a borrowed value against a prepared type and complete it into an owned value,
/// filling only the exact record fields omitted by that record's own defaults. Defaults are
/// already complete in the prepared fields, and a supplied `None` is never replaced.
pub(super) fn validate_and_complete_value(
    value: &Value,
    ty: &PreparedType,
    limits: &Limits,
) -> Result<Value, MonError> {
    let mut path = Vec::new();
    let mut budget = BudgetState::new(limits);
    complete_value(
        value,
        ty,
        &mut budget,
        &mut path,
        0,
        CompletionContext::Input,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionContext {
    Default,
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordContext {
    Record,
    Choice,
}

fn prepare_type(
    schema_type: SchemaType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<PreparedType, MonError> {
    if let Err(error) = check_depth(budget, depth, path) {
        drop_schema_type_tree(schema_type);
        return Err(error);
    }
    if let Err(error) = charge_nodes(budget, 1, path) {
        drop_schema_type_tree(schema_type);
        return Err(error);
    }

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
            if let Err(error) = charge_decoded_bytes(budget, name.len(), path) {
                drop_fields_tree(fields);
                return Err(error);
            }
            if let Err(error) = validate_identifier(&name, path, "struct name") {
                drop_fields_tree(fields);
                return Err(error);
            }
            Ok(PreparedType::Struct {
                name,
                fields: prepare_fields(fields, budget, path, true, depth)?,
            })
        }

        SchemaType::Collection { element } => Ok(PreparedType::Collection {
            element: Box::new(prepare_type(*element, budget, path, depth + 1)?),
        }),

        SchemaType::Map { key, value } => {
            let key = match prepare_type(*key, budget, path, depth + 1) {
                Ok(prepared) => prepared,
                Err(error) => {
                    drop_schema_type_tree(*value);
                    return Err(error);
                }
            };
            if !is_supported_map_key(&key) {
                drop_schema_type_tree(*value);
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
            if let Err(error) = charge_decoded_bytes(budget, name.len(), path) {
                drop_variants_tree(variants);
                return Err(error);
            }
            if let Err(error) = validate_identifier(&name, path, "choice name") {
                drop_variants_tree(variants);
                return Err(error);
            }
            let mut prepared_variants = Vec::new();
            let mut variants = variants.into_iter();

            while let Some(variant) = variants.next() {
                if let Err(error) = charge_decoded_bytes(budget, variant.name.len(), path) {
                    drop_variant_tree(variant);
                    for remaining in variants {
                        drop_variant_tree(remaining);
                    }
                    return Err(error);
                }
                if let Err(error) = charge_nodes(budget, 1, path) {
                    drop_variant_tree(variant);
                    for remaining in variants {
                        drop_variant_tree(remaining);
                    }
                    return Err(error);
                }
                if prepared_variants
                    .iter()
                    .any(|known: &PreparedVariant| known.name == variant.name)
                {
                    let mut variant_path = path.clone();
                    variant_path.push(PathSegment::Variant(variant.name.clone()));
                    let duplicate_name = variant.name.clone();
                    drop_variant_tree(variant);
                    for remaining in variants {
                        drop_variant_tree(remaining);
                    }
                    return Err(schema_error(
                        MonErrorCode::InvalidSchema,
                        &variant_path,
                        format!(
                            "duplicate choice variant '{duplicate_name}'; names must be unique"
                        ),
                    ));
                }

                let Variant {
                    name: variant_name,
                    fields: variant_fields,
                } = variant;
                path.push(PathSegment::Variant(variant_name.clone()));
                let prepared_fields = if let Err(error) =
                    validate_identifier(&variant_name, path, "choice variant name")
                {
                    drop_fields_tree(variant_fields);
                    Err(error)
                } else {
                    prepare_fields(variant_fields, budget, path, false, depth)
                };
                path.pop();
                match prepared_fields {
                    Ok(fields) => prepared_variants.push(PreparedVariant {
                        name: variant_name,
                        fields,
                    }),
                    Err(error) => {
                        for remaining in variants {
                            drop_variant_tree(remaining);
                        }
                        return Err(error);
                    }
                }
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
    let mut fields = fields.into_iter();

    while let Some(field) = fields.next() {
        if let Err(error) = charge_nodes(budget, 1, path) {
            drop_field_into_stack(field);
            for remaining in fields {
                drop_field_into_stack(remaining);
            }
            return Err(error);
        }
        if let Err(error) = validate_identifier(&field.name, path, "field name") {
            drop_field_into_stack(field);
            for remaining in fields {
                drop_field_into_stack(remaining);
            }
            return Err(error);
        }
        if prepared_fields
            .iter()
            .any(|known: &PreparedField| known.name == field.name)
        {
            if let Err(error) = charge_decoded_bytes(budget, field.name.len(), path) {
                drop_field_into_stack(field);
                for remaining in fields {
                    drop_field_into_stack(remaining);
                }
                return Err(error);
            }
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(field.name.clone()));
            let duplicate_name = field.name.clone();
            drop_field_into_stack(field);
            for remaining in fields {
                drop_field_into_stack(remaining);
            }
            return Err(schema_error(
                MonErrorCode::DuplicateField,
                &field_path,
                format!("duplicate field '{duplicate_name}'; names must be unique"),
            ));
        }

        if !allow_defaults && field.default.is_some() {
            if let Err(error) = charge_decoded_bytes(budget, field.name.len(), path) {
                drop_field_into_stack(field);
                for remaining in fields {
                    drop_field_into_stack(remaining);
                }
                return Err(error);
            }
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(field.name.clone()));
            drop_field_into_stack(field);
            for remaining in fields {
                drop_field_into_stack(remaining);
            }
            return Err(schema_error(
                MonErrorCode::InvalidSchema,
                &field_path,
                "choice payload fields cannot have defaults",
            ));
        }

        match prepare_field(field, budget, path, allow_defaults, depth + 1) {
            Ok(prepared) => prepared_fields.push(prepared),
            Err(error) => {
                for remaining in fields {
                    drop_field_into_stack(remaining);
                }
                return Err(error);
            }
        }
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

    if let Err(error) = charge_decoded_bytes(budget, name.len(), path) {
        drop_schema_type_tree(ty);
        if let Some(default) = default {
            drop_value_tree(default);
        }
        return Err(error);
    }
    path.push(PathSegment::Field(name.clone()));
    let ty = match prepare_type(ty, budget, path, depth) {
        Ok(prepared) => prepared,
        Err(error) => {
            path.pop();
            if let Some(default) = default {
                drop_value_tree(default);
            }
            return Err(error);
        }
    };
    let default = match default {
        Some(value) => {
            if !allow_defaults {
                path.pop();
                drop_value_tree(value);
                return Err(schema_error(
                    MonErrorCode::InvalidSchema,
                    path,
                    "choice payload fields cannot have defaults",
                ));
            }
            match complete_value(&value, &ty, budget, path, depth, CompletionContext::Default) {
                Ok(completed) => Some(completed),
                Err(error) => {
                    path.pop();
                    drop_value_tree(value);
                    return Err(error);
                }
            }
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

fn key_value_string_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        _ => 0,
    }
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
    value: &Value,
    ty: &PreparedType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
    context: CompletionContext,
) -> Result<Value, MonError> {
    check_depth(budget, depth, path)?;
    if let PreparedType::Optional(inner) = ty {
        if matches!(value, Value::None) {
            charge_nodes(budget, 1, path)?;
            return Ok(Value::None);
        }
        return complete_value(value, inner, budget, path, depth, context);
    }
    charge_nodes(budget, 1, path)?;

    match (value, ty) {
        (Value::None, PreparedType::None) => Ok(Value::None),
        (Value::None, _) => Err(value_error(
            context,
            MonErrorCode::TypeMismatch,
            path,
            "none is not accepted by this schema type",
        )),

        (Value::Bool(value), PreparedType::Bool) => Ok(Value::Bool(*value)),
        (Value::Int(value), PreparedType::Int) => Ok(Value::Int(*value)),
        (Value::Char(value), PreparedType::Char) => {
            charge_decoded_bytes(budget, value.len_utf8(), path)?;
            Ok(Value::Char(*value))
        }
        (Value::String(value), PreparedType::String) => {
            charge_decoded_bytes(budget, value.len(), path)?;
            Ok(Value::String(value.clone()))
        }
        (Value::Float(value), PreparedType::Float) => {
            if value.is_finite() {
                Ok(Value::Float(*value))
            } else {
                Err(value_error(
                    context,
                    MonErrorCode::NonFiniteFloat,
                    path,
                    "Float values must be finite",
                ))
            }
        }

        (Value::Integer(value), PreparedType::Integer) => {
            // Caller-owned output copy stays charged here; normalization scratch is
            // charged inside validation before the shared helper reserves it.
            charge_decoded_bytes(budget, value.len(), path)?;
            validate_integer_text(value, budget, path, context)?;
            Ok(Value::Integer(value.clone()))
        }
        (Value::Decimal(value), PreparedType::Decimal { scale }) => {
            // Caller-owned output copy stays charged here; normalization scratch is
            // charged inside validation before the shared helper reserves it.
            charge_decoded_bytes(budget, value.len(), path)?;
            validate_decimal_text(value, *scale, budget, path, context)?;
            Ok(Value::Decimal(value.clone()))
        }

        (Value::Record(values), PreparedType::Record { fields })
        | (Value::Record(values), PreparedType::Struct { fields, .. }) => {
            let values = complete_record(
                values,
                fields,
                budget,
                path,
                depth,
                context,
                RecordContext::Record,
            )?;
            Ok(Value::Record(values))
        }

        (Value::Collection(values), PreparedType::Collection { element }) => {
            let mut completed = Vec::new();
            for (index, value) in values.iter().enumerate() {
                path.push(PathSegment::Index(index));
                let value = complete_value(value, element, budget, path, depth + 1, context)?;
                path.pop();
                completed.push(value);
            }
            Ok(Value::Collection(completed))
        }

        (Value::Map(entries), PreparedType::Map { key, value }) => {
            let mut completed = Vec::new();
            let mut seen = MapKeyIndex::new();
            for (index, (key_value, value_value)) in entries.iter().enumerate() {
                path.push(PathSegment::Index(index));
                let key_value = complete_value(key_value, key, budget, path, depth + 1, context)?;
                let Some(is_duplicate) = seen.contains_value(&key_value) else {
                    path.pop();
                    return Err(value_error(
                        context,
                        MonErrorCode::InvalidMapKey,
                        path,
                        "MON map key type is not supported",
                    ));
                };
                if is_duplicate {
                    charge_decoded_bytes(budget, super::map_key_name_len(&key_value), path)?;
                    let key_name = super::map_key_name(&key_value);
                    path.pop();
                    path.push(PathSegment::MapKey(key_name));
                    return Err(value_error(
                        context,
                        MonErrorCode::DuplicateMapKey,
                        path,
                        "MON map contains a duplicate decoded key",
                    ));
                }
                // Charge the validation-only index before it can grow. Strings are cloned
                // only after their additional storage is included in the decoded-byte budget.
                charge_nodes(budget, 1, path)?;
                if matches!(&key_value, Value::String(_)) {
                    charge_decoded_bytes(budget, key_value_string_len(&key_value), path)?;
                }
                seen.insert_value(&key_value);
                let value_value =
                    complete_value(value_value, value, budget, path, depth + 1, context)?;
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
            if let Some(qualifier) = qualifier {
                if !is_identifier(qualifier) {
                    return Err(value_error(
                        context,
                        MonErrorCode::InvalidIdentifier,
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
            if let Some(qualifier) = qualifier {
                charge_decoded_bytes(budget, qualifier.len(), path)?;
            }
            charge_decoded_bytes(budget, variant.len(), path)?;
            let Some(expected_variant) =
                variants.iter().find(|candidate| candidate.name == *variant)
            else {
                return Err(schema_error(
                    MonErrorCode::UnknownVariant,
                    path,
                    format!("unknown choice variant '{variant}'"),
                ));
            };

            let qualifier = qualifier.clone();
            let variant_name = variant.clone();
            path.push(PathSegment::Variant(variant_name.clone()));
            let fields = complete_record(
                values,
                &expected_variant.fields,
                budget,
                path,
                depth,
                context,
                RecordContext::Choice,
            )?;
            path.pop();

            Ok(Value::Choice {
                qualifier,
                variant: variant_name,
                fields,
            })
        }

        (Value::Collection(_), PreparedType::Map { .. })
        | (Value::Map(_), PreparedType::Collection { .. }) => Err(value_error(
            context,
            MonErrorCode::MapKind,
            path,
            "collection and map values are distinct MON container kinds",
        )),
        (_, _) => Err(value_error(
            context,
            MonErrorCode::TypeMismatch,
            path,
            "value does not match its receiving schema type",
        )),
    }
}
fn complete_record(
    values: &[(String, Value)],
    fields: &[PreparedField],
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
    context: CompletionContext,
    record_context: RecordContext,
) -> Result<Vec<(String, Value)>, MonError> {
    // Schema preparation already bounded this per-record routing table. Charging its
    // field slots for every value would double-count repeated records during encoding.
    let mut supplied: Vec<Option<&Value>> = (0..fields.len()).map(|_| None).collect();

    for (name, value) in values {
        if !is_identifier(name) {
            charge_decoded_bytes(budget, name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(name.clone()));
            return Err(value_error(
                context,
                MonErrorCode::InvalidIdentifier,
                &field_path,
                "record field name is not a valid identifier",
            ));
        }

        let Some(index) = fields.iter().position(|field| field.name == *name) else {
            charge_decoded_bytes(budget, name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(name.clone()));
            let code = if matches!(context, CompletionContext::Input)
                && matches!(record_context, RecordContext::Choice)
            {
                MonErrorCode::UnknownArgument
            } else {
                MonErrorCode::UnknownField
            };
            return Err(schema_error(
                code,
                &field_path,
                format!("unknown field '{name}'"),
            ));
        };

        if supplied[index].is_some() {
            charge_decoded_bytes(budget, name.len(), path)?;
            let mut field_path = path.clone();
            field_path.push(PathSegment::Field(name.clone()));
            let code = if matches!(context, CompletionContext::Input)
                && matches!(record_context, RecordContext::Choice)
            {
                MonErrorCode::DuplicateArgument
            } else {
                MonErrorCode::DuplicateField
            };
            return Err(schema_error(
                code,
                &field_path,
                format!("duplicate field '{name}'"),
            ));
        }
        supplied[index] = Some(value);
    }

    let mut completed = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        charge_decoded_bytes(budget, field.name.len(), path)?;
        path.push(PathSegment::Field(field.name.clone()));
        let value = match supplied[index].take() {
            Some(value) => complete_value(value, &field.ty, budget, path, depth + 1, context)?,
            None => match &field.default {
                Some(default) => {
                    charge_default(budget, path)?;
                    clone_default(default, budget, path, depth + 1)?
                }
                None => {
                    let code = if matches!(record_context, RecordContext::Choice) {
                        MonErrorCode::Arity
                    } else {
                        MonErrorCode::MissingField
                    };
                    return Err(value_error(
                        context,
                        code,
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
    budget: &mut BudgetState<'_>,
    path: &[PathSegment],
    context: CompletionContext,
) -> Result<(), MonError> {
    check_numeric_text_budget(text, budget.limits, path)?;
    // The shared helper reserves capacity for the decoded unsigned-token bytes, so
    // charge that owned scratch before parsing. Malformed tokens still allocate it.
    let unsigned = unsigned_numeric_text(text);
    charge_decoded_bytes(budget, unsigned.len(), path)?;
    let parsed = parse_numeric_literal(unsigned)
        .map_err(|reason| numeric_value_error(path, reason, context))?;
    check_numeric_budget(parsed.digit_count as usize, budget.limits, path)?;

    if parsed.kind != NumericLiteralKind::WholeNumber {
        return Err(value_error(
            context,
            MonErrorCode::NumericType,
            path,
            "Integer requires whole-number spelling",
        ));
    }

    Ok(())
}

fn validate_decimal_text(
    text: &str,
    scale: u8,
    budget: &mut BudgetState<'_>,
    path: &[PathSegment],
    context: CompletionContext,
) -> Result<(), MonError> {
    check_numeric_text_budget(text, budget.limits, path)?;
    // The shared helper reserves capacity for the decoded unsigned-token bytes, so
    // charge that owned scratch before parsing. Malformed tokens still allocate it.
    let unsigned = unsigned_numeric_text(text);
    charge_decoded_bytes(budget, unsigned.len(), path)?;
    let parsed = parse_numeric_literal(unsigned)
        .map_err(|reason| numeric_value_error(path, reason, context))?;
    check_numeric_budget(parsed.digit_count as usize, budget.limits, path)?;

    let (coefficient, exponent_magnitude, negative_exponent) =
        match parsed.normalized_text.find('e') {
            None => (parsed.normalized_text.as_str(), 0, false),
            Some(separator) => {
                let coefficient = &parsed.normalized_text[..separator];
                let exponent = &parsed.normalized_text[(separator + 1)..];
                let negative_exponent = exponent.starts_with('-');
                let digits = exponent
                    .strip_prefix('+')
                    .or_else(|| exponent.strip_prefix('-'))
                    .unwrap_or(exponent);
                (
                    coefficient,
                    saturating_decimal_usize(digits),
                    negative_exponent,
                )
            }
        };
    let effective = decimal_effective_scale(coefficient, exponent_magnitude, negative_exponent);
    if effective > scale as usize {
        return Err(schema_error(
            MonErrorCode::NumericScale,
            path,
            format!("Decimal value has effective scale {effective}, above declared scale {scale}"),
        ));
    }
    Ok(())
}

/// Borrowed effective-scale calculation shared by reader and programmatic validation.
///
/// WHY: both paths must apply the same exact-decimal scale policy without a second
///      arithmetic runtime or a mantissa-digit copy. The inputs borrow the normalized
///      literal (already stripped of separators and the exponent separator), so this
///      helper allocates nothing. Each caller keeps its own error context.
pub(super) fn decimal_effective_scale(
    coefficient: &str,
    exponent_magnitude: usize,
    negative_exponent: bool,
) -> usize {
    let (integer_part, fractional_part) = match coefficient.split_once('.') {
        Some((integer_part, fractional_part)) => (integer_part, fractional_part),
        None => (coefficient, ""),
    };
    let fractional_digits = fractional_part.len();
    let mut all_zero = true;
    let mut trailing_zeroes = 0usize;
    let mut seen_nonzero = false;
    // Borrow the coefficient digits in reverse so trailing zeroes are counted without
    // copying the mantissa into a temporary vector.
    for character in integer_part.bytes().chain(fractional_part.bytes()).rev() {
        if character == b'0' {
            if !seen_nonzero {
                trailing_zeroes += 1;
            }
        } else {
            all_zero = false;
            seen_nonzero = true;
        }
    }
    if all_zero {
        return 0;
    }

    // Keep the full host-sized scale for diagnostics. Exponent parsing and scale
    // arithmetic saturate at usize::MAX without allocating arbitrary precision.
    let untrimmed_scale = if negative_exponent {
        fractional_digits.saturating_add(exponent_magnitude)
    } else {
        fractional_digits.saturating_sub(exponent_magnitude)
    };
    let trailing_zeroes = trailing_zeroes.min(untrimmed_scale);
    untrimmed_scale.saturating_sub(trailing_zeroes)
}

fn unsigned_numeric_text(text: &str) -> &str {
    text.strip_prefix('-').unwrap_or(text)
}

pub(super) fn saturating_decimal_usize(text: &str) -> usize {
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
                "MON numeric digit budget exceeded (limit {})",
                limits.max_numeric_digits
            ),
        ));
    }
    Ok(())
}

fn numeric_value_error(
    path: &[PathSegment],
    reason: NumberLiteralErrorReason,
    context: CompletionContext,
) -> MonError {
    let code = match reason {
        NumberLiteralErrorReason::OutsideIntRange => MonErrorCode::NumericRange,
        NumberLiteralErrorReason::NonFiniteFloat => MonErrorCode::NonFiniteFloat,
        _ => MonErrorCode::NumericSyntax,
    };
    value_error(
        context,
        code,
        path,
        format!("invalid MON numeric literal ({reason:?})"),
    )
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn value_error(
    context: CompletionContext,
    code: MonErrorCode,
    path: &[PathSegment],
    detail: impl Into<String>,
) -> MonError {
    match context {
        CompletionContext::Default => invalid_default(path, detail),
        CompletionContext::Input => schema_error(code, path, detail),
    }
}

fn invalid_default(path: &[PathSegment], detail: impl Into<String>) -> MonError {
    schema_error(MonErrorCode::InvalidDefault, path, detail)
}

fn schema_error(code: MonErrorCode, path: &[PathSegment], detail: impl Into<String>) -> MonError {
    MonError::new(code, None, path, detail)
}

/// Iteratively release an unvisited `SchemaType` tree without recursion.
///
/// Preparation moves rejected schema values out of their owners before
/// returning an error (duplicate names, invalid identifiers, map keys,
/// invalid defaults and budget failures). A recursive drop of an arbitrarily
/// deep untouched tail could overflow native stack on cleanup, so rejection
/// paths route through this explicit work-list first.
fn drop_schema_type_tree(root: SchemaType) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            SchemaType::Optional(inner) => stack.push(*inner),
            SchemaType::Record { fields } | SchemaType::Struct { fields, .. } => {
                drop_fields_into_stack(fields, &mut stack)
            }
            SchemaType::Collection { element } => stack.push(*element),
            SchemaType::Map { key, value } => {
                stack.push(*key);
                stack.push(*value);
            }
            SchemaType::Choice { variants, .. } => {
                for variant in variants {
                    drop_variant_into_stack(variant, &mut stack);
                }
            }
            SchemaType::None
            | SchemaType::Bool
            | SchemaType::Char
            | SchemaType::String
            | SchemaType::Int
            | SchemaType::Float
            | SchemaType::Integer
            | SchemaType::Decimal { .. }
            | SchemaType::Unsupported { .. } => {}
        }
    }
}

fn drop_fields_tree(fields: Vec<Field>) {
    for field in fields {
        drop_field_into_stack(field);
    }
}

fn drop_variants_tree(variants: Vec<Variant>) {
    for variant in variants {
        drop_variant_tree(variant);
    }
}

/// Iteratively release an unvisited `Value` default tree without recursion.
///
/// Rejected malformed defaults still own their untouched tail; dropping that
/// tail recursively could exceed native stack on cleanup.
fn drop_value_tree(root: Value) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Record(fields) => {
                for (_, value) in fields.into_iter().rev() {
                    stack.push(value);
                }
            }
            Value::Collection(values) => {
                for value in values.into_iter().rev() {
                    stack.push(value);
                }
            }
            Value::Map(entries) => {
                for (key, value) in entries.into_iter().rev() {
                    stack.push(key);
                    stack.push(value);
                }
            }
            Value::Choice { fields, .. } => {
                for (_, value) in fields.into_iter().rev() {
                    stack.push(value);
                }
            }
            Value::None
            | Value::Bool(_)
            | Value::Char(_)
            | Value::String(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Integer(_)
            | Value::Decimal(_) => {}
        }
    }
}

fn drop_fields_into_stack(fields: Vec<Field>, stack: &mut Vec<SchemaType>) {
    for field in fields.into_iter().rev() {
        let Field { ty, default, .. } = field;
        if let Some(default) = default {
            drop_value_tree(default);
        }
        stack.push(ty);
    }
}

fn drop_variant_into_stack(variant: Variant, stack: &mut Vec<SchemaType>) {
    let Variant { fields, .. } = variant;
    drop_fields_into_stack(fields, stack);
}

fn drop_field_into_stack(field: Field) {
    let Field { ty, default, .. } = field;
    if let Some(default) = default {
        drop_value_tree(default);
    }
    drop_schema_type_tree(ty);
}

fn drop_variant_tree(variant: Variant) {
    let Variant { fields, .. } = variant;
    drop_fields_tree(fields);
}

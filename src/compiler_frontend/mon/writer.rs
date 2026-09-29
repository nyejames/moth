//! Literal-only MON writers.
//!
//! The writer validates and completes a borrowed value into an owned value before rendering it.
//! Rendering is kept deliberately separate from validation so a caller only receives a complete
//! `String` on success; a failure never exposes the private output buffer.

use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::numeric_scalar::BinaryFloatPrecision;
use crate::compiler_frontend::numeric_text::format::format_finite_float;

use super::schema::{PreparedType, validate_and_complete_value};
use super::{MonError, MonErrorCode, PathSegment, PreparedSchema, Value};

/// Whitespace policy for the private writer service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WriteOptions {
    pub(crate) pretty: bool,
}

impl WriteOptions {
    pub(crate) const COMPACT: Self = Self { pretty: false };
    #[cfg(test)]
    pub(crate) const PRETTY: Self = Self { pretty: true };
}

/// Encode a complete root-record document in compact form.
pub fn encode_document(value: &Value, schema: &PreparedSchema) -> Result<String, MonError> {
    encode_document_with_options(value, schema, WriteOptions::COMPACT)
}

/// Encode a nested value in compact form.
pub fn encode_value(value: &Value, schema: &PreparedSchema) -> Result<String, MonError> {
    encode_value_with_options(value, schema, WriteOptions::COMPACT)
}

/// Encode a complete root-record document with the requested whitespace policy.
pub(crate) fn encode_document_with_options(
    value: &Value,
    schema: &PreparedSchema,
    options: WriteOptions,
) -> Result<String, MonError> {
    if !matches!(
        schema.root,
        PreparedType::Record { .. } | PreparedType::Struct { .. }
    ) {
        return Err(MonError::new(
            MonErrorCode::RootNotRecord,
            None,
            &[],
            "prepared MON root schema is not a record",
        ));
    }
    if !matches!(value, Value::Record(_)) {
        return Err(MonError::new(
            MonErrorCode::RootNotRecord,
            None,
            &[],
            "MON document root must be a record",
        ));
    }

    let completed = validate_and_complete_value(value, &schema.root, &schema.limits)?;
    let mut writer = Writer::new(&schema.limits, options);
    writer.write_value(&completed, &schema.root, 0)?;
    Ok(writer.output)
}

/// Encode a nested value with the requested whitespace policy.
pub(crate) fn encode_value_with_options(
    value: &Value,
    schema: &PreparedSchema,
    options: WriteOptions,
) -> Result<String, MonError> {
    let completed = validate_and_complete_value(value, &schema.root, &schema.limits)?;
    let mut writer = Writer::new(&schema.limits, options);
    writer.write_value(&completed, &schema.root, 0)?;
    Ok(writer.output)
}

struct Writer<'a> {
    limits: &'a super::Limits,
    options: WriteOptions,
    output: String,
    path: Vec<PathSegment>,
}

impl<'a> Writer<'a> {
    fn new(limits: &'a super::Limits, options: WriteOptions) -> Self {
        Self {
            limits,
            options,
            output: String::new(),
            path: Vec::new(),
        }
    }

    fn write_value(
        &mut self,
        value: &Value,
        ty: &PreparedType,
        depth: usize,
    ) -> Result<(), MonError> {
        // Validation has already checked this depth, but keep the rendering walk bounded as well.
        if depth > self.limits.max_depth {
            return self.fail(
                MonErrorCode::DepthBudget,
                format!(
                    "MON nesting depth exceeded (limit {})",
                    self.limits.max_depth
                ),
            );
        }

        if let PreparedType::Optional(inner) = ty {
            if matches!(value, Value::None) {
                return self.push_str("none");
            }
            return self.write_value(value, inner, depth);
        }

        match (value, ty) {
            (Value::None, PreparedType::None) => self.push_str("none"),
            (Value::Bool(value), PreparedType::Bool) => {
                self.push_str(if *value { "true" } else { "false" })
            }
            (Value::Char(value), PreparedType::Char) => self.write_char(*value),
            (Value::String(value), PreparedType::String) => self.write_string(value),
            (Value::Int(value), PreparedType::Int { .. }) => self.write_number(&value.to_string()),
            (Value::Float(value), PreparedType::Float { precision }) => {
                self.write_float(*value, BinaryFloatPrecision::from(*precision))
            }
            (Value::Integer(value), PreparedType::Integer)
            | (Value::Decimal(value), PreparedType::Decimal { .. }) => self.write_number(value),
            (value, PreparedType::Fixed(scalar)) => self.write_fixed(value, *scalar),
            (
                Value::Record(fields),
                PreparedType::Record {
                    fields: schema_fields,
                }
                | PreparedType::Struct {
                    fields: schema_fields,
                    ..
                },
            ) => self.write_record(fields, schema_fields, depth),
            (Value::Collection(values), PreparedType::Collection { element }) => {
                self.write_collection(values, element, depth)
            }
            (
                Value::Map(entries),
                PreparedType::Map {
                    key,
                    value: value_ty,
                },
            ) => self.write_map(entries, key, value_ty, depth),
            (
                Value::Choice {
                    qualifier: _,
                    variant,
                    fields,
                },
                PreparedType::Choice { variants, .. },
            ) => self.write_choice(variant, fields, variants, depth),
            // `validate_and_complete_value` makes this unreachable.  Keep a structured guard here
            // so a future prepared-schema change cannot make the writer silently emit bad data.
            _ => self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON value did not match its prepared schema",
            ),
        }
    }

    fn write_record(
        &mut self,
        fields: &[(String, Value)],
        schema_fields: &super::schema::PreparedFields,
        depth: usize,
    ) -> Result<(), MonError> {
        if fields.len() != schema_fields.len() {
            return self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON record field count did not match its schema",
            );
        }
        self.push_str("(")?;
        if fields.is_empty() {
            return self.push_str(")");
        }
        if self.options.pretty {
            self.push_str("\n")?;
        }
        for (index, ((name, value), schema_field)) in
            fields.iter().zip(schema_fields.iter()).enumerate()
        {
            if name != &schema_field.name {
                return self.fail(
                    MonErrorCode::InternalInvariant,
                    "validated MON record field order did not match its schema",
                );
            }
            if index > 0 {
                self.push_str(",")?;
                if self.options.pretty {
                    self.push_str("\n")?;
                } else {
                    self.push_str(" ")?;
                }
            }
            if self.options.pretty {
                self.indent(depth + 1)?;
            }
            self.push_str(&schema_field.name)?;
            self.push_str(" = ")?;
            self.path
                .push(PathSegment::Field(schema_field.name.clone()));
            self.write_value(value, &schema_field.ty, depth + 1)?;
            self.path.pop();
        }
        if self.options.pretty {
            self.push_str("\n")?;
            self.indent(depth)?;
        }
        self.push_str(")")
    }

    fn write_collection(
        &mut self,
        values: &[Value],
        element: &PreparedType,
        depth: usize,
    ) -> Result<(), MonError> {
        self.push_str("{")?;
        if values.is_empty() {
            return self.push_str("}");
        }
        if self.options.pretty {
            self.push_str("\n")?;
        }
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                self.push_str(",")?;
                if self.options.pretty {
                    self.push_str("\n")?;
                } else {
                    self.push_str(" ")?;
                }
            }
            if self.options.pretty {
                self.indent(depth + 1)?;
            }
            self.path.push(PathSegment::Index(index));
            self.write_value(value, element, depth + 1)?;
            self.path.pop();
        }
        if self.options.pretty {
            self.push_str("\n")?;
            self.indent(depth)?;
        }
        self.push_str("}")
    }

    fn write_map(
        &mut self,
        entries: &[(Value, Value)],
        key: &PreparedType,
        value_ty: &PreparedType,
        depth: usize,
    ) -> Result<(), MonError> {
        self.push_str("{")?;
        if entries.is_empty() {
            self.push_str("=")?;
            return self.push_str("}");
        }
        if self.options.pretty {
            self.push_str("\n")?;
        }
        for (index, (key_value, value)) in entries.iter().enumerate() {
            if index > 0 {
                self.push_str(",")?;
                if self.options.pretty {
                    self.push_str("\n")?;
                } else {
                    self.push_str(" ")?;
                }
            }
            if self.options.pretty {
                self.indent(depth + 1)?;
            }
            self.path.push(PathSegment::Index(index));
            self.write_value(key_value, key, depth + 1)?;
            self.push_str(" = ")?;
            self.write_value(value, value_ty, depth + 1)?;
            self.path.pop();
        }
        if self.options.pretty {
            self.push_str("\n")?;
            self.indent(depth)?;
        }
        self.push_str("}")
    }

    fn write_choice(
        &mut self,
        variant: &str,
        fields: &[(String, Value)],
        variants: &super::schema::PreparedVariants,
        depth: usize,
    ) -> Result<(), MonError> {
        let Some(expected_index) = variants.find(variant) else {
            return self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON choice variant was not found in its schema",
            );
        };
        let expected = &variants[expected_index];
        if fields.len() != expected.fields.len() {
            return self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON choice field count did not match its schema",
            );
        }
        self.push_str("::")?;
        self.push_str(variant)?;
        if fields.is_empty() {
            return Ok(());
        }
        self.push_str("(")?;
        if self.options.pretty {
            self.push_str("\n")?;
        }
        for (index, ((name, value), schema_field)) in
            fields.iter().zip(expected.fields.iter()).enumerate()
        {
            if name != &schema_field.name {
                return self.fail(
                    MonErrorCode::InternalInvariant,
                    "validated MON choice field order did not match its schema",
                );
            }
            if index > 0 {
                self.push_str(",")?;
                if self.options.pretty {
                    self.push_str("\n")?;
                } else {
                    self.push_str(" ")?;
                }
            }
            if self.options.pretty {
                self.indent(depth + 1)?;
            }
            self.push_str(&schema_field.name)?;
            self.push_str(" = ")?;
            self.path.push(PathSegment::Variant(variant.to_owned()));
            self.path
                .push(PathSegment::Field(schema_field.name.clone()));
            self.write_value(value, &schema_field.ty, depth + 1)?;
            self.path.pop();
            self.path.pop();
        }
        if self.options.pretty {
            self.push_str("\n")?;
            self.indent(depth)?;
        }
        self.push_str(")")
    }

    fn write_number(&mut self, text: &str) -> Result<(), MonError> {
        self.check_numeric_digits(text)?;
        self.push_str(text)
    }

    /// Write one explicit-width scalar or `Byte` value.
    ///
    /// WHY: completion already checked the value's family and materialised a binary float at its
    ///      own precision, so this only renders the payload. A structured internal-invariant guard
    ///      keeps a future prepared-schema change from silently emitting another width's value.
    fn write_fixed(&mut self, value: &Value, scalar: FixedScalar) -> Result<(), MonError> {
        if value.fixed_scalar() != Some(scalar) {
            return self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON value did not match its prepared schema",
            );
        }

        match value {
            Value::I8(payload) => self.write_number(&payload.to_string()),
            Value::I16(payload) => self.write_number(&payload.to_string()),
            Value::I32(payload) => self.write_number(&payload.to_string()),
            Value::I64(payload) => self.write_number(&payload.to_string()),
            Value::U8(payload) => self.write_number(&payload.to_string()),
            Value::U16(payload) => self.write_number(&payload.to_string()),
            Value::U32(payload) => self.write_number(&payload.to_string()),
            Value::U64(payload) => self.write_number(&payload.to_string()),
            Value::Byte(payload) => self.write_number(&payload.to_string()),
            Value::F16(payload) => self.write_float(*payload, BinaryFloatPrecision::Binary16),
            Value::F32(payload) => self.write_float(*payload, BinaryFloatPrecision::Binary32),
            Value::F64(payload) => self.write_float(*payload, BinaryFloatPrecision::Binary64),
            _ => self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON value did not match its prepared schema",
            ),
        }
    }

    /// Write one finite binary-float value at its own precision.
    ///
    /// Negative zero keeps its sign as `-0.0` because the shared formatter normalises it to `0`,
    /// which would lose a value the reader can distinguish. Every other value uses the shared
    /// shortest round-trippable text for its precision, so an `F16`/`F32` value never prints more
    /// digits than its own precision carries.
    fn write_float(&mut self, value: f64, precision: BinaryFloatPrecision) -> Result<(), MonError> {
        if !value.is_finite() {
            return self.fail(MonErrorCode::NonFiniteFloat, "Float values must be finite");
        }
        let text = if value == 0.0 && value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            format_finite_float(value, precision).map_err(|_| {
                MonError::new(
                    MonErrorCode::NonFiniteFloat,
                    None,
                    &self.path,
                    "Float values must be finite",
                )
            })?
        };
        self.write_number(&text)
    }

    fn write_string(&mut self, value: &str) -> Result<(), MonError> {
        self.push_str("\"")?;
        self.write_escaped(value, '"')?;
        self.push_str("\"")
    }

    fn write_char(&mut self, value: char) -> Result<(), MonError> {
        self.push_str("'")?;
        self.write_escaped(&value.to_string(), '\'')?;
        self.push_str("'")
    }

    fn write_escaped(&mut self, value: &str, quote: char) -> Result<(), MonError> {
        for character in value.chars() {
            match character {
                '\\' => self.push_str("\\\\")?,
                '\n' => self.push_str("\\n")?,
                '\r' => self.push_str("\\r")?,
                '\t' => self.push_str("\\t")?,
                character if character == quote => {
                    self.push_char('\\')?;
                    self.push_char(character)?;
                }
                character if character.is_control() => {
                    self.push_str(&format!("\\u{{{:X}}}", character as u32))?;
                }
                character => self.push_char(character)?,
            }
        }
        Ok(())
    }

    fn check_numeric_digits(&self, text: &str) -> Result<(), MonError> {
        let digits = text.bytes().filter(u8::is_ascii_digit).count();
        if digits > self.limits.max_numeric_digits {
            return self.fail(
                MonErrorCode::NumericBudget,
                format!(
                    "MON numeric digit budget exceeded (limit {})",
                    self.limits.max_numeric_digits
                ),
            );
        }
        Ok(())
    }

    fn indent(&mut self, depth: usize) -> Result<(), MonError> {
        for _ in 0..depth {
            self.push_str("    ")?;
        }
        Ok(())
    }

    fn push_char(&mut self, value: char) -> Result<(), MonError> {
        let length = value.len_utf8();
        self.ensure_output(length)?;
        self.output.push(value);
        Ok(())
    }

    fn push_str(&mut self, value: &str) -> Result<(), MonError> {
        self.ensure_output(value.len())?;
        self.output.push_str(value);
        Ok(())
    }

    fn ensure_output(&self, additional: usize) -> Result<(), MonError> {
        let Some(next) = self.output.len().checked_add(additional) else {
            return self.fail(
                MonErrorCode::OutputBudget,
                "MON output budget arithmetic overflowed",
            );
        };
        if next > self.limits.max_output_bytes {
            return self.fail(
                MonErrorCode::OutputBudget,
                format!(
                    "MON output exceeds byte budget (limit {})",
                    self.limits.max_output_bytes
                ),
            );
        }
        Ok(())
    }

    fn fail<T>(&self, code: MonErrorCode, detail: impl Into<String>) -> Result<T, MonError> {
        Err(MonError::new(code, None, &self.path, detail))
    }
}

#[cfg(test)]
#[path = "tests/writer_tests.rs"]
mod tests;

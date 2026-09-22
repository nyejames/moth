//! Literal-only MON writers.
//!
//! The writer validates and completes a borrowed value into an owned value before rendering it.
//! Rendering is kept deliberately separate from validation so a caller only receives a complete
//! `String` on success; a failure never exposes the private output buffer.

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
            (Value::Int(value), PreparedType::Int) => self.write_number(&value.to_string()),
            (Value::Float(value), PreparedType::Float) => self.write_float(*value),
            (Value::Integer(value), PreparedType::Integer)
            | (Value::Decimal(value), PreparedType::Decimal { .. }) => self.write_number(value),
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
        schema_fields: &[super::schema::PreparedField],
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
        variants: &[super::schema::PreparedVariant],
        depth: usize,
    ) -> Result<(), MonError> {
        let Some(expected) = variants.iter().find(|candidate| candidate.name == variant) else {
            return self.fail(
                MonErrorCode::InternalInvariant,
                "validated MON choice variant was not found in its schema",
            );
        };
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

    fn write_float(&mut self, value: f64) -> Result<(), MonError> {
        if !value.is_finite() {
            return self.fail(MonErrorCode::NonFiniteFloat, "Float values must be finite");
        }
        let text = if value == 0.0 && value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            format_finite_float(value).map_err(|_| {
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
mod tests {
    use super::*;
    use crate::compiler_frontend::mon::{
        Field, Limits, Schema, SchemaType, Variant, decode_document,
    };

    fn schema(fields: Vec<Field>) -> PreparedSchema {
        Schema::record(fields)
            .prepare()
            .expect("test schema prepares")
    }

    #[test]
    fn compact_writer_completes_defaults_and_orders_fields() {
        let schema = schema(vec![
            Field::with_default("second", SchemaType::Int, Value::Int(2)),
            Field::required("first", SchemaType::String),
        ]);
        let value = Value::Record(vec![("first".into(), Value::String("x".into()))]);
        let encoded = encode_document(&value, &schema).expect("value encodes");
        assert_eq!(encoded, "(second = 2, first = \"x\")");
        let decoded = decode_document(&encoded, &schema).expect("encoded document decodes");
        assert_eq!(
            decoded,
            Value::Record(vec![
                ("second".into(), Value::Int(2)),
                ("first".into(), Value::String("x".into())),
            ])
        );
    }

    #[test]
    fn exact_numbers_unicode_signed_zero_and_empty_containers_round_trip() {
        let schema = schema(vec![
            Field::required("integer", SchemaType::Integer),
            Field::required("decimal", SchemaType::Decimal { scale: 3 }),
            Field::required("float", SchemaType::Float),
            Field::required("boundary", SchemaType::Float),
            Field::required("text", SchemaType::String),
            Field::required("letter", SchemaType::Char),
            Field::required("quote", SchemaType::Char),
            Field::required("slash", SchemaType::Char),
            Field::required("control", SchemaType::Char),
            Field::required(
                "items",
                SchemaType::Collection {
                    element: Box::new(SchemaType::Int),
                },
            ),
            Field::required(
                "map",
                SchemaType::Map {
                    key: Box::new(SchemaType::String),
                    value: Box::new(SchemaType::Int),
                },
            ),
        ]);
        let value = Value::Record(vec![
            (
                "integer".into(),
                Value::Integer("90071992547409931234567890".into()),
            ),
            ("decimal".into(), Value::Decimal("1.230".into())),
            ("float".into(), Value::Float(-0.0)),
            ("boundary".into(), Value::Float(f64::MIN_POSITIVE)),
            ("text".into(), Value::String("\"\\\n\r\t\u{0001}".into())),
            ("letter".into(), Value::Char('😀')),
            ("quote".into(), Value::Char('\'')),
            ("slash".into(), Value::Char('\\')),
            ("control".into(), Value::Char('\n')),
            ("items".into(), Value::Collection(Vec::new())),
            ("map".into(), Value::Map(Vec::new())),
        ]);
        let encoded = encode_document(&value, &schema).expect("value encodes");
        assert!(encoded.contains("-0.0"));
        assert!(encoded.contains("\\\""));
        assert!(encoded.contains("\\\\"));
        assert!(encoded.contains("\\n"));
        assert!(encoded.contains("\\r"));
        assert!(encoded.contains("\\t"));
        assert!(encoded.contains("\\u{1}"));
        assert!(encoded.contains("items = {}"));
        assert!(encoded.contains("map = {=}"));
        let decoded = decode_document(&encoded, &schema).expect("encoded document decodes");
        assert_eq!(decoded, value);
        let Value::Record(decoded_fields) = &decoded else {
            panic!("writer produced a non-record");
        };
        let decoded_float = decoded_fields
            .iter()
            .find(|(name, _)| name == "float")
            .and_then(|(_, value)| match value {
                Value::Float(value) => Some(*value),
                _ => None,
            })
            .expect("decoded signed zero field");
        assert_eq!(decoded_float.to_bits(), (-0.0f64).to_bits());
        let decoded_boundary = decoded_fields
            .iter()
            .find(|(name, _)| name == "boundary")
            .and_then(|(_, value)| match value {
                Value::Float(value) => Some(*value),
                _ => None,
            })
            .expect("decoded finite boundary field");
        assert_eq!(decoded_boundary.to_bits(), f64::MIN_POSITIVE.to_bits());
    }

    #[test]
    fn choices_maps_and_pretty_output_have_same_semantics() {
        let schema = schema(vec![
            Field::required(
                "theme",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![
                        Variant::unit("Light"),
                        Variant::payload(
                            "Custom",
                            vec![Field::required("name", SchemaType::String)],
                        ),
                    ],
                },
            ),
            Field::required(
                "scores",
                SchemaType::Map {
                    key: Box::new(SchemaType::String),
                    value: Box::new(SchemaType::Int),
                },
            ),
        ]);
        let value = Value::Record(vec![
            (
                "theme".into(),
                Value::Choice {
                    qualifier: Some("Theme".into()),
                    variant: "Custom".into(),
                    fields: vec![("name".into(), Value::String("Gold".into()))],
                },
            ),
            (
                "scores".into(),
                Value::Map(vec![
                    (Value::String("a".into()), Value::Int(1)),
                    (Value::String("b".into()), Value::Int(2)),
                ]),
            ),
        ]);
        let compact = encode_document(&value, &schema).expect("compact encoding");
        let pretty = encode_document_with_options(&value, &schema, WriteOptions::PRETTY)
            .expect("pretty encoding");
        assert_eq!(
            decode_document(&compact, &schema).unwrap(),
            decode_document(&pretty, &schema).unwrap()
        );
        assert_eq!(
            compact
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>(),
            pretty
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>(),
        );
        assert!(pretty.contains('\n'));
        assert!(compact.contains("::Custom(name = \"Gold\")"));
        assert!(compact.contains("\"a\" = 1, \"b\" = 2"));
    }

    #[test]
    fn writer_rejects_bad_values_and_output_budget_without_partial_result() {
        let schema = schema(vec![Field::required("value", SchemaType::Int)]);
        let wrong = encode_document(
            &Value::Record(vec![("value".into(), Value::String("bad".into()))]),
            &schema,
        )
        .unwrap_err();
        assert_eq!(wrong.code, MonErrorCode::TypeMismatch);

        let bounded_string_schema = Schema::value(SchemaType::String)
            .with_limits(Limits {
                max_decoded_bytes: 2,
                ..Limits::default()
            })
            .prepare()
            .expect("bounded string schema prepares");
        let decoded_error =
            encode_value(&Value::String("too long".into()), &bounded_string_schema).unwrap_err();
        assert_eq!(decoded_error.code, MonErrorCode::DecodedBudget);

        let tiny = Schema::record(vec![Field::required("value", SchemaType::String)])
            .with_limits(Limits {
                max_output_bytes: 2,
                ..Limits::default()
            })
            .prepare()
            .unwrap();
        let budget = encode_document(
            &Value::Record(vec![("value".into(), Value::String("long".into()))]),
            &tiny,
        )
        .unwrap_err();
        assert_eq!(budget.code, MonErrorCode::OutputBudget);
    }

    #[test]
    fn nested_fragments_compose_into_a_complete_document() {
        let nested = Schema::value(SchemaType::Struct {
            name: "Window".into(),
            fields: vec![
                Field::required("width", SchemaType::Int),
                Field::with_default("height", SchemaType::Int, Value::Int(720)),
            ],
        })
        .prepare()
        .expect("nested schema prepares");
        let fragment = encode_value(
            &Value::Record(vec![("width".into(), Value::Int(1280))]),
            &nested,
        )
        .expect("nested value encodes");
        assert_eq!(fragment, "(width = 1280, height = 720)");

        let root = schema(vec![Field::required(
            "window",
            SchemaType::Struct {
                name: "Window".into(),
                fields: vec![
                    Field::required("width", SchemaType::Int),
                    Field::required("height", SchemaType::Int),
                ],
            },
        )]);
        let document = format!("window = {fragment}");
        let decoded = decode_document(&document, &root).expect("composed document decodes");
        assert_eq!(
            decoded,
            Value::Record(vec![(
                "window".into(),
                Value::Record(vec![
                    ("width".into(), Value::Int(1280)),
                    ("height".into(), Value::Int(720)),
                ]),
            )])
        );
    }

    #[test]
    fn writer_rejects_qualifier_numeric_float_depth_and_node_errors() {
        let choice_schema = schema(vec![Field::required(
            "theme",
            SchemaType::Choice {
                name: "Theme".into(),
                variants: vec![
                    Variant::unit("Light"),
                    Variant::payload("Custom", vec![Field::required("name", SchemaType::String)]),
                ],
            },
        )]);
        let qualifier_error = encode_document(
            &Value::Record(vec![(
                "theme".into(),
                Value::Choice {
                    qualifier: Some("Other".into()),
                    variant: "Light".into(),
                    fields: Vec::new(),
                },
            )]),
            &choice_schema,
        )
        .unwrap_err();
        assert_eq!(qualifier_error.code, MonErrorCode::QualifierMismatch);

        let integer_schema = schema(vec![Field::required("value", SchemaType::Integer)]);
        let numeric_error = encode_document(
            &Value::Record(vec![("value".into(), Value::Integer("1e3".into()))]),
            &integer_schema,
        )
        .unwrap_err();
        assert_eq!(numeric_error.code, MonErrorCode::NumericType);

        let float_schema = schema(vec![Field::required("value", SchemaType::Float)]);
        let float_error = encode_document(
            &Value::Record(vec![("value".into(), Value::Float(f64::NAN))]),
            &float_schema,
        )
        .unwrap_err();
        assert_eq!(float_error.code, MonErrorCode::NonFiniteFloat);
        let depth_error = Schema::value(SchemaType::Collection {
            element: Box::new(SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            }),
        })
        .with_limits(Limits {
            max_depth: 1,
            ..Limits::default()
        })
        .prepare()
        .unwrap_err();
        assert_eq!(depth_error.code, MonErrorCode::DepthBudget);

        let malformed_depth_schema = Schema::value(SchemaType::Int)
            .with_limits(Limits {
                max_depth: 1,
                ..Limits::default()
            })
            .prepare()
            .expect("scalar schema prepares");
        let malformed_depth_error = encode_value(
            &Value::Collection(vec![Value::Collection(vec![Value::Int(1)])]),
            &malformed_depth_schema,
        )
        .unwrap_err();
        assert_eq!(malformed_depth_error.code, MonErrorCode::TypeMismatch);
        let map_schema = Schema::value(SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        })
        .prepare()
        .expect("map schema prepares");
        let duplicate_map_error = encode_value(
            &Value::Map(vec![
                (Value::String("same".into()), Value::Int(1)),
                (Value::String("same".into()), Value::Int(2)),
            ]),
            &map_schema,
        )
        .unwrap_err();
        assert_eq!(duplicate_map_error.code, MonErrorCode::DuplicateMapKey);

        let arity_error = encode_document(
            &Value::Record(vec![(
                "theme".into(),
                Value::Choice {
                    qualifier: None,
                    variant: "Custom".into(),
                    fields: Vec::new(),
                },
            )]),
            &choice_schema,
        )
        .unwrap_err();
        assert_eq!(arity_error.code, MonErrorCode::Arity);

        let node_schema = Schema::record(vec![Field::required(
            "values",
            SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            },
        )])
        .with_limits(Limits {
            max_nodes: 4,
            ..Limits::default()
        })
        .prepare()
        .expect("schema preparation fits node budget");
        let node_error = encode_document(
            &Value::Record(vec![(
                "values".into(),
                Value::Collection(vec![Value::Int(1), Value::Int(2)]),
            )]),
            &node_schema,
        )
        .unwrap_err();
        assert_eq!(node_error.code, MonErrorCode::NodeBudget);
    }

    #[test]
    fn nested_scalar_encoder_returns_complete_literal() {
        let string_schema = Schema::value(SchemaType::String)
            .prepare()
            .expect("string schema prepares");
        assert_eq!(
            encode_value(&Value::String("looks = MON".into()), &string_schema).unwrap(),
            "\"looks = MON\""
        );

        let optional_schema = Schema::value(SchemaType::Optional(Box::new(SchemaType::String)))
            .prepare()
            .expect("optional schema prepares");
        assert_eq!(
            encode_value(&Value::None, &optional_schema).unwrap(),
            "none"
        );
    }
}

//! Expression lowering helpers for the JavaScript backend.
//!
//! These routines map HIR expressions into JS source strings while preserving the backend's
//! binding and alias helper conventions.

use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::{
    JsNumericCarrier, JsNumericConversion, binary_float_precision_bits,
};
use crate::backends::js::value_use::JsValueUse;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::{
    HirMapEntryRange, HirProjection, HirVariantFieldRange,
};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirVariantCarrier};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::places::HirPlace;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
#[derive(Clone, Copy)]
enum OptionComparisonSide {
    Option { inner_type: TypeId },
    NoneLiteral,
    Other { type_id: TypeId },
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn lower_expr(
        &mut self,
        expression_id: HirValueId,
    ) -> Result<String, InfrastructureFailure> {
        let hir = self.hir;
        let expression = hir.expressions.expression(expression_id);
        // WHAT: dispatch lowering by the fully resolved HIR expression shape.
        // WHY: HIR has already linearized side effects, so expression lowering can stay a direct
        //      semantic mapping from each variant to the exact JS runtime helper sequence it needs.
        match &expression.kind {
            HirExpressionKind::FixedScalar(value) => JsNumericCarrier::fixed_literal(*value)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::BackendLowering,
                        format!("JS backend cannot format fixed scalar literal {value}"),
                    )
                }),
            HirExpressionKind::Number(value) => Ok(JsNumericCarrier::number_literal(value)),
            HirExpressionKind::VariantConstruct {
                carrier,
                variant_index,
                fields,
            } => self.lower_variant_construct(carrier, *variant_index, *fields),
            HirExpressionKind::RouteConstruct {
                choice_id,
                variant_index,
                fields,
                ..
            } => self.lower_variant_construct(
                &HirVariantCarrier::Choice {
                    choice_id: *choice_id,
                },
                *variant_index,
                *fields,
            ),

            HirExpressionKind::Bool(value) => Ok(value.to_string()),
            HirExpressionKind::Char(value) => Ok(escape_js_char(*value)),
            HirExpressionKind::StringLiteral(value) => Ok(escape_js_string(value)),
            HirExpressionKind::StructuralString { pieces } => {
                self.lower_structural_string(hir.expressions.string_pieces(*pieces))
            }

            HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => {
                self.lower_expression_for_use(expression_id, JsValueUse::PlainExpression)
            }

            HirExpressionKind::BinOp { left, op, right } => self.lower_bin_op(*left, *op, *right),
            HirExpressionKind::UnaryOp { op, operand } => self.lower_unary_op(*op, *operand),

            HirExpressionKind::StructConstruct { fields, .. } => {
                let fields = hir.expressions.struct_fields(*fields);
                let mut pairs = Vec::with_capacity(fields.len());
                for (field_id, value) in fields {
                    let field_name = self.field_name(*field_id)?.to_owned();
                    let field_value = self.lower_expr(*value)?;
                    pairs.push(format!("{field_name}: {field_value}"));
                }

                Ok(format!("{{ {} }}", pairs.join(", ")))
            }

            HirExpressionKind::Collection(elements) => {
                let lowered = hir
                    .expressions
                    .values(*elements)
                    .iter()
                    .map(|element| self.lower_expr(*element))
                    .collect::<Result<Vec<_>, _>>()?;

                let items = format!("[{}]", lowered.join(", "));

                let Some(collection_shape) = self.type_environment.collection_shape(expression.ty)
                else {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JS backend lowered a collection expression whose type is not a collection"
                    );
                };

                if let Some(fixed_capacity) = collection_shape.fixed_capacity {
                    // Capacities are source-visible I32 counts, so their emitted literal uses the
                    // exact I32 carrier (a decimal Number).
                    let capacity = i64::from(i32::try_from(fixed_capacity).map_err(|_| {
                        InfrastructureFailure::internal(
                            CompilerStage::BackendLowering,
                            "JS fixed collection capacity exceeds the I32 capacity carrier",
                        )
                    })?);
                    let capacity = FixedScalarValue::signed(FixedScalar::I32, capacity)
                        .ok_or_else(|| {
                            InfrastructureFailure::internal(
                                CompilerStage::BackendLowering,
                                "JS fixed collection capacity exceeds the I32 capacity carrier",
                            )
                        })?;
                    let fixed_capacity =
                        JsNumericCarrier::fixed_literal(capacity).ok_or_else(|| {
                            InfrastructureFailure::internal(
                                CompilerStage::BackendLowering,
                                "JS fixed collection capacity exceeds the I32 capacity carrier",
                            )
                        })?;
                    Ok(format!(
                        "__moth_fixed_collection({}, {})",
                        items, fixed_capacity
                    ))
                } else {
                    Ok(items)
                }
            }

            HirExpressionKind::MapLiteral(entries) => {
                self.lower_map_literal(expression.ty, *entries)
            }

            HirExpressionKind::Range { start, end } => {
                let start = self.lower_expr(*start)?;
                let end = self.lower_expr(*end)?;
                Ok(format!("{{ start: {start}, end: {end} }}"))
            }

            HirExpressionKind::Cast { source, policy } => {
                let lowered_source = self.lower_expr(*source)?;
                js_cast_expression_for_policy(*policy, &lowered_source)
            }

            HirExpressionKind::VariantPayloadGet {
                carrier,
                source,
                variant_index,
                field_index,
            } => self.lower_variant_payload_get(carrier, *source, *variant_index, *field_index),
        }
    }

    fn lower_structural_string(
        &self,
        pieces: &[ConstStringPiece],
    ) -> Result<String, InfrastructureFailure> {
        let Some(url_map) = self.config.structural_string_urls.as_ref() else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::BackendLowering,
                "JavaScript lowering received a structural string without a builder URL map",
            ));
        };

        let mut rendered = String::new();
        for piece in pieces {
            match piece {
                ConstStringPiece::Text(text) => rendered.push_str(self.string_table.resolve(*text)),
                ConstStringPiece::Resource(resource_id) => {
                    let Some(url) = url_map.resource_urls.get(resource_id) else {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::BackendLowering,
                            format!(
                                "JavaScript lowering has no rendered URL for structural resource {resource_id:?}"
                            ),
                        ));
                    };
                    rendered.push_str(url);
                }
                ConstStringPiece::SiteRoot => {
                    let Some(url) = url_map.site_root_url.as_deref() else {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::BackendLowering,
                            "JavaScript lowering has no rendered URL for a structural site root",
                        ));
                    };
                    rendered.push_str(url);
                }
            }
        }

        Ok(escape_js_string(&rendered))
    }

    // ------------------
    //  Place lowering
    // ------------------

    /// Lower an HIR place expression to a JS runtime field/index access expression.
    ///
    /// WHAT: maps `HirPlace` variants (local, field, index) to the corresponding JS runtime
    /// helper calls (`__moth_field`, `__moth_index`) or a direct local name.
    /// WHY: the JS backend uses runtime helpers for field and index access to preserve its
    /// reference and aliasing semantics.
    pub(crate) fn lower_place(
        &mut self,
        place: &HirPlace,
    ) -> Result<String, InfrastructureFailure> {
        let hir = self.hir;
        let mut lowered = self.local_name(place.root)?.to_owned();
        for projection in hir.expressions.projections(place.projections) {
            match *projection {
                HirProjection::Field(field_id) => {
                    let field = escape_js_string(self.field_name(field_id)?);
                    lowered = format!("__moth_field({lowered}, {field})");
                }
                HirProjection::Index(index_id) => {
                    let index = self.lower_expr(index_id)?;
                    lowered = format!("__moth_index({lowered}, {index})");
                }
            }
        }
        Ok(lowered)
    }

    // ------------------------
    //  Variant construction
    // ------------------------

    // WHAT: lowers a variant construction into a JS object literal.
    // WHY: centralises tag policy and field-key escaping in one place.
    fn lower_variant_construct(
        &mut self,
        carrier: &HirVariantCarrier,
        variant_index: usize,
        fields: HirVariantFieldRange,
    ) -> Result<String, InfrastructureFailure> {
        let hir = self.hir;
        let mut entries = vec![];
        for field in hir.expressions.variant_fields(fields) {
            let js_value = self.lower_expr(field.value)?;
            if let Some(name) = field.name {
                let js_name = escape_js_string(self.string_table.resolve(name));
                entries.push(format!("{js_name}: {js_value}"));
            } else {
                entries.push(js_value);
            }
        }

        let tag_entry = match carrier {
            HirVariantCarrier::Choice { .. } => format!("tag: {variant_index}"),
            HirVariantCarrier::Option => {
                let tag = if variant_index == 0 { "none" } else { "some" };
                format!("tag: \"{tag}\"")
            }
        };

        let mut all_entries = vec![tag_entry];
        all_entries.extend(entries);
        Ok(format!("{{ {} }}", all_entries.join(", ")))
    }

    // WHAT: lowers a variant payload field access into a JS bracket-access expression.
    // WHY: bracket access is safe for field names that collide with JS reserved words.
    fn lower_variant_payload_get(
        &mut self,
        carrier: &HirVariantCarrier,
        source: HirValueId,
        variant_index: usize,
        field_index: usize,
    ) -> Result<String, InfrastructureFailure> {
        let source_js = self.lower_expr(source)?;
        let field_name_js = match carrier {
            HirVariantCarrier::Choice { choice_id } => {
                let choice = self.hir.choices.get(choice_id.0 as usize).unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JavaScript backend: invalid ChoiceId {choice_id:?} in VariantPayloadGet"
                    )
                });
                let variant = choice.variants.get(variant_index).unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JavaScript backend: invalid variant index {variant_index} for choice {choice_id:?}"
                    )
                });
                let field = variant.fields.get(field_index).unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JavaScript backend: invalid field index {field_index} for variant {variant_index} of choice {choice_id:?}"
                    )
                });
                escape_js_string(self.string_table.resolve(field.name))
            }
            HirVariantCarrier::Option => "\"value\"".to_owned(),
        };
        Ok(format!("({source_js})[{field_name_js}]"))
    }

    /// Whether a resolved type is a nominal choice type.
    ///
    /// WHY: choice carriers are object literals with a `tag` property, so equality
    /// must compare tags rather than using reference equality.
    fn is_choice_type_id(&self, type_id: TypeId) -> bool {
        self.type_environment.variants_for(type_id).is_some()
    }

    // ----------------------------
    //  Binary operator lowering
    // ----------------------------

    fn lower_bin_op(
        &mut self,
        left: HirValueId,
        operator: HirBinOp,
        right: HirValueId,
    ) -> Result<String, InfrastructureFailure> {
        let hir = self.hir;
        let left_type = hir.expressions.expression(left).ty;
        let right_type = hir.expressions.expression(right).ty;
        let option_equality = if matches!(operator, HirBinOp::Eq | HirBinOp::Ne) {
            self.option_equality_sides(left_type, right_type)
        } else {
            None
        };

        let left = self.lower_expr(left)?;
        let right = self.lower_expr(right)?;

        if let Some((left_side, right_side)) = option_equality {
            return self.lower_option_equality(left, left_side, operator, right, right_side);
        }

        if matches!(operator, HirBinOp::Eq | HirBinOp::Ne) {
            let equality = self.lower_typed_equality(left, left_type, right, right_type);
            return match operator {
                HirBinOp::Eq => Ok(equality),
                HirBinOp::Ne => Ok(format!("(!{equality})")),
                _ => crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS equality dispatch already restricted this arm to HirBinOp::Eq or HirBinOp::Ne"
                ),
            };
        }

        // Relational Number/BigInt operators compare exact integer values directly without
        // coercing either carrier to the other. StringAppend remains ordinary JS concatenation.
        let js_operator = match operator {
            HirBinOp::StringAppend => "+",
            HirBinOp::Eq | HirBinOp::Ne => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS equality/inequality already returned at the Eq/Ne dispatch above this relational match"
            ),
            HirBinOp::Lt => "<",
            HirBinOp::Le => "<=",
            HirBinOp::Gt => ">",
            HirBinOp::Ge => ">=",
            // Short-circuit runtime semantics are guaranteed by HIR CFG lowering:
            // `and`/`or` may arrive here as plain BinOp only when both operands are side-effect
            // free at expression level. Branch-gated RHS evaluation is lowered earlier.
            HirBinOp::And => "&&",
            HirBinOp::Or => "||",
        };

        Ok(format!("({left} {js_operator} {right})"))
    }

    fn option_equality_sides(
        &self,
        left_type: TypeId,
        right_type: TypeId,
    ) -> Option<(OptionComparisonSide, OptionComparisonSide)> {
        let left_side = self.classify_option_comparison_side(left_type);
        let right_side = self.classify_option_comparison_side(right_type);

        if matches!(left_side, OptionComparisonSide::Option { .. })
            || matches!(right_side, OptionComparisonSide::Option { .. })
        {
            Some((left_side, right_side))
        } else {
            None
        }
    }

    fn classify_option_comparison_side(&self, type_id: TypeId) -> OptionComparisonSide {
        let Some(inner_type) = self.type_environment.option_inner_type(type_id) else {
            return OptionComparisonSide::Other { type_id };
        };

        if inner_type == self.type_environment.builtins().none {
            return OptionComparisonSide::NoneLiteral;
        }

        OptionComparisonSide::Option { inner_type }
    }

    fn lower_option_equality(
        &mut self,
        left: String,
        left_side: OptionComparisonSide,
        operator: HirBinOp,
        right: String,
        right_side: OptionComparisonSide,
    ) -> Result<String, InfrastructureFailure> {
        let equality = match (left_side, right_side) {
            (OptionComparisonSide::Option { .. }, OptionComparisonSide::NoneLiteral) => {
                format!("(({left}).tag === \"none\")")
            }

            (OptionComparisonSide::NoneLiteral, OptionComparisonSide::Option { .. }) => {
                format!("(({right}).tag === \"none\")")
            }

            (
                OptionComparisonSide::Option {
                    inner_type: left_type,
                },
                OptionComparisonSide::Option {
                    inner_type: right_type,
                },
            ) => {
                let inner_equality = self.lower_typed_equality(
                    format!("({left}).value"),
                    left_type,
                    format!("({right}).value"),
                    right_type,
                );
                format!(
                    "((({left}).tag === ({right}).tag) && ((({left}).tag === \"none\") || {inner_equality}))"
                )
            }

            (
                OptionComparisonSide::Option { inner_type },
                OptionComparisonSide::Other { type_id },
            ) => {
                let inner_equality = self.lower_typed_equality(
                    format!("({left}).value"),
                    inner_type,
                    right,
                    type_id,
                );
                format!("((({left}).tag === \"some\") && {inner_equality})")
            }

            (
                OptionComparisonSide::Other { type_id },
                OptionComparisonSide::Option { inner_type },
            ) => {
                let inner_equality = self.lower_typed_equality(
                    left,
                    type_id,
                    format!("({right}).value"),
                    inner_type,
                );
                format!("((({right}).tag === \"some\") && {inner_equality})")
            }

            _ => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend received option equality without an option operand"
            ),
        };

        match operator {
            HirBinOp::Eq => Ok(equality),
            HirBinOp::Ne => Ok(format!("(!{equality})")),
            _ => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend received non-equality option comparison"
            ),
        }
    }

    fn numeric_type_uses_bigint_carrier(&self, type_id: TypeId) -> Option<bool> {
        if self.type_environment.fixed_scalar(type_id) == Some(FixedScalar::Byte) {
            return Some(false);
        }

        let scalar = NumericScalar::from_type_id(type_id, self.type_environment)?;
        let carrier = JsNumericCarrier::for_scalar(scalar)?;
        Some(matches!(
            carrier,
            JsNumericCarrier::BigInteger { .. } | JsNumericCarrier::ScaledInteger { .. }
        ))
    }

    /// Lower equality according to the semantic types of both operands.
    ///
    /// WHAT: selects language equality for String and choice carriers and loosens only numeric
    ///       equality whose JS carriers differ, preserving exact Number/BigInt comparisons.
    /// WHY: option equality can compare an inner numeric value with a different fixed-width type,
    ///      so carrier identity must be checked at the actual value boundary.
    pub(crate) fn lower_typed_equality(
        &mut self,
        left: String,
        left_type: TypeId,
        right: String,
        right_type: TypeId,
    ) -> String {
        if self.is_choice_type_id(left_type) && self.is_choice_type_id(right_type) {
            self.used_choice_equality = true;
            return format!("__moth_choice_eq({left}, {right})");
        }

        let numeric_carriers_differ = match (
            self.numeric_type_uses_bigint_carrier(left_type),
            self.numeric_type_uses_bigint_carrier(right_type),
        ) {
            (Some(left_bigint), Some(right_bigint)) => left_bigint != right_bigint,
            _ => false,
        };
        let equality = if numeric_carriers_differ { "==" } else { "===" };

        format!("({left} {equality} {right})")
    }

    // ---------------------------
    //  Unary operator lowering
    // ---------------------------

    /// Lower consecutive unary operators without recursive lowering or repeated child copies.
    ///
    /// Each operator keeps its parenthesized layer. Other expression kinds remain owned by
    /// `lower_expr`, so their conversions, precedence and lowering effects are unchanged.
    fn lower_unary_op(
        &mut self,
        mut operator: HirUnaryOp,
        mut operand: HirValueId,
    ) -> Result<String, InfrastructureFailure> {
        let mut rendered = String::new();
        let mut closing_parentheses = 0;

        loop {
            rendered.push_str(match operator {
                HirUnaryOp::Neg => "(-",
                HirUnaryOp::Not => "(!",
            });
            closing_parentheses += 1;

            let expression = self.hir.expressions.expression(operand);
            let HirExpressionKind::UnaryOp {
                op,
                operand: next_operand,
            } = &expression.kind
            else {
                break;
            };
            operator = *op;
            operand = *next_operand;
        }

        let terminal_operand = self.lower_expr(operand)?;
        rendered.reserve(terminal_operand.len() + closing_parentheses);
        rendered.push_str(&terminal_operand);
        for _ in 0..closing_parentheses {
            rendered.push(')');
        }

        Ok(rendered)
    }

    // -----------------------
    //  Map literal lowering
    // -----------------------

    /// Lower a map literal expression into a `__moth_map_new` call.
    ///
    /// WHAT: converts each `HirMapEntry` into a `[key, value]` pair and wraps the array in
    /// `__moth_map_new` so the runtime constructs a branded ordered-map wrapper.
    /// WHY: map literals are first-class compiler-owned values; the backend must not emit
    ///      raw JS `Map` constructors because the runtime helper layer owns the branded shape.
    fn lower_map_literal(
        &mut self,
        type_id: TypeId,
        entries: HirMapEntryRange,
    ) -> Result<String, InfrastructureFailure> {
        let Some(_map_shape) = self.type_environment.map_shape(type_id) else {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend lowered a map literal whose type is not a map"
            );
        };

        let hir = self.hir;
        let entries = hir.expressions.map_entries(entries);
        let mut lowered_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let key = self.lower_expr(entry.key)?;
            let value = self.lower_expr(entry.value)?;
            lowered_entries.push(format!("[{key}, {value}]"));
        }

        Ok(format!("__moth_map_new([{}])", lowered_entries.join(", ")))
    }
}

// -----------------
//  Escape helpers
// -----------------

pub(crate) fn escape_js_string(value: &str) -> String {
    let mut escaped = String::from("\"");

    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            control if control.is_control() => {
                escaped.push_str(&format!("\\u{:04X}", control as u32));
            }
            normal => escaped.push(normal),
        }
    }
    escaped.push('"');
    escaped
}

/// Returns the JavaScript expression implementing one builtin cast policy.
pub(super) fn js_cast_expression_for_policy(
    policy: BuiltinCastPolicyId,
    value: &str,
) -> Result<String, InfrastructureFailure> {
    match policy {
        BuiltinCastPolicyId::NumericConversion { source, target } => {
            JsNumericConversion::classify(source, target)
                .map(|conversion| conversion.expression(value, source, target))
        }
        BuiltinCastPolicyId::ByteToU8 | BuiltinCastPolicyId::U8ToByte => Ok(value.to_owned()),
        BuiltinCastPolicyId::NumericToString(NumericScalar::Number(scale)) => {
            Ok(format!("__moth_format_number({value}, {})", scale.get()))
        }
        BuiltinCastPolicyId::NumericToString(scalar) if scalar.is_integer() => {
            Ok(format!("__moth_cast_int_to_string({value})"))
        }
        BuiltinCastPolicyId::NumericToString(scalar) => {
            let precision = scalar.binary_float_precision().unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS backend received a non-numeric string formatting policy for {scalar}"
                )
            });
            Ok(format!(
                "__moth_cast_float_to_string({value}, {}, \"{scalar}\")",
                binary_float_precision_bits(precision),
            ))
        }
        BuiltinCastPolicyId::BoolToString => Ok(format!("__moth_cast_bool_to_string({value})")),
        BuiltinCastPolicyId::CharToString => Ok(format!("__moth_cast_char_to_string({value})")),
        BuiltinCastPolicyId::CharToInt => Ok(format!("__moth_cast_char_to_int({value})")),
        BuiltinCastPolicyId::StringToError => Ok(format!("__moth_cast_string_to_error({value})")),
        BuiltinCastPolicyId::ErrorToString => Ok(format!("__moth_cast_error_to_string({value})")),
        BuiltinCastPolicyId::IntToChar => Ok(format!("__moth_cast_int_to_char({value})")),
        BuiltinCastPolicyId::StringToNumeric(scalar @ NumericScalar::Number(scale)) => Ok(format!(
            "__moth_cast_number({value}, {}, \"{scalar}\")",
            scale.get()
        )),
        BuiltinCastPolicyId::StringToNumeric(scalar) if scalar.is_integer() => {
            let carrier = JsNumericCarrier::for_scalar(scalar).unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS backend has no integer carrier for {scalar}"
                )
            });
            let (minimum, maximum) = carrier.integer_bounds_js().unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS backend has no integer bounds for {scalar}"
                )
            });
            Ok(format!(
                "__moth_cast_int({value}, {minimum}, {maximum}, \"{scalar}\")",
            ))
        }
        BuiltinCastPolicyId::StringToNumeric(scalar) => {
            let precision = scalar.binary_float_precision().unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS backend received a non-numeric parse policy for {scalar}"
                )
            });
            Ok(format!(
                "__moth_cast_float({value}, {}, \"{scalar}\")",
                binary_float_precision_bits(precision),
            ))
        }
        BuiltinCastPolicyId::StringToBool => Ok(format!("__moth_cast_string_to_bool({value})")),
        BuiltinCastPolicyId::StringToChar => Ok(format!("__moth_cast_string_to_char({value})")),
    }
}

fn escape_js_char(value: char) -> String {
    let mut escaped = String::from("\"");

    match value {
        '\\' => escaped.push_str("\\\\"),
        '"' => escaped.push_str("\\\""),
        '\n' => escaped.push_str("\\n"),
        '\r' => escaped.push_str("\\r"),
        '\t' => escaped.push_str("\\t"),
        '\0' => escaped.push_str("\\0"),
        control if control.is_control() => {
            escaped.push_str(&format!("\\u{:04X}", control as u32));
        }
        normal => escaped.push(normal),
    }

    escaped.push('"');
    escaped
}

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
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantCarrier,
};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::places::HirPlace;
#[derive(Clone, Copy)]
enum OptionComparisonSide {
    Option { inner_type: TypeId },
    NoneLiteral,
    Other { type_id: TypeId },
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn lower_fallible_success_condition(
        &mut self,
        result: &HirExpression,
    ) -> Result<String, CompilerError> {
        let lowered_result = self.lower_expr(result)?;

        Ok(format!("(({lowered_result}).tag === \"ok\")"))
    }

    pub(crate) fn lower_expr(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        // Reactive template values have language type `String` but need a backend-owned runtime
        // representation. In ordinary expression contexts we snapshot them to a plain string.
        if self.value_is_reactive_template(expression.id) {
            let template_value = self.lower_reactive_template_value(expression)?;
            return Ok(format!("__moth_template_snapshot({template_value})"));
        }

        self.lower_expr_without_reactive_snapshot(expression)
    }

    // ----------------------
    //  Expression dispatch
    // ----------------------

    fn lower_expr_without_reactive_snapshot(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        // WHAT: dispatch lowering by the fully resolved HIR expression shape.
        // WHY: HIR has already linearized side effects, so expression lowering can stay a direct
        //      semantic mapping from each variant to the exact JS runtime helper sequence it needs.
        match &expression.kind {
            HirExpressionKind::Int(value) => {
                JsNumericCarrier::int_literal(*value, self.config.numeric_profile).ok_or_else(
                    || {
                        CompilerError::compiler_error(
                            "JS backend cannot format an Int literal for its numeric profile",
                        )
                    },
                )
            }
            HirExpressionKind::FixedScalar(value) => {
                JsNumericCarrier::fixed_literal(*value, self.config.numeric_profile).ok_or_else(
                    || {
                        CompilerError::compiler_error(format!(
                            "JS backend cannot format fixed scalar literal {value}"
                        ))
                    },
                )
            }
            HirExpressionKind::VariantConstruct {
                carrier,
                variant_index,
                fields,
            } => self.lower_variant_construct(carrier, *variant_index, fields),
            // HIR Float literals already contain the exact value rounded to the profile precision.
            // Non-finite values remain an internal invariant violation.
            HirExpressionKind::Float(value) => {
                let carrier =
                    JsNumericCarrier::for_scalar(NumericScalar::Float, self.config.numeric_profile)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "JS backend has no carrier for Float literals",
                            )
                        })?;
                if carrier.float_precision().is_none() {
                    return Err(CompilerError::compiler_error(
                        "JS backend mapped a Float literal to an integer carrier",
                    ));
                }
                if !value.is_finite() {
                    return Err(CompilerError::compiler_error(
                        "JavaScript backend received non-finite HIR Float literal",
                    ));
                }

                Ok(value.to_string())
            }

            HirExpressionKind::Bool(value) => Ok(value.to_string()),
            HirExpressionKind::Char(value) => Ok(escape_js_char(*value)),
            HirExpressionKind::StringLiteral(value) => Ok(escape_js_string(value)),
            HirExpressionKind::StructuralString { pieces } => self.lower_structural_string(pieces),

            HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => {
                self.lower_expression_for_use(expression, JsValueUse::PlainExpression)
            }

            HirExpressionKind::BinOp { left, op, right } => self.lower_bin_op(left, *op, right),
            HirExpressionKind::UnaryOp { op, operand } => self.lower_unary_op(*op, operand),

            HirExpressionKind::StructConstruct { fields, .. } => {
                let mut pairs = Vec::with_capacity(fields.len());
                for (field_id, value) in fields {
                    let field_name = self.field_name(*field_id)?.to_owned();
                    let field_value = self.lower_expr(value)?;
                    pairs.push(format!("{field_name}: {field_value}"));
                }

                Ok(format!("{{ {} }}", pairs.join(", ")))
            }

            HirExpressionKind::Collection(elements) => {
                let lowered = elements
                    .iter()
                    .map(|element| self.lower_expr(element))
                    .collect::<Result<Vec<_>, _>>()?;

                let items = format!("[{}]", lowered.join(", "));

                let Some(collection_shape) = self.type_environment.collection_shape(expression.ty)
                else {
                    return Err(CompilerError::compiler_error(
                        "JS backend lowered a collection expression whose type is not a collection",
                    ));
                };

                if let Some(fixed_capacity) = collection_shape.fixed_capacity {
                    let fixed_capacity = i64::try_from(fixed_capacity).map_err(|_| {
                        CompilerError::compiler_error(
                            "JS fixed collection capacity exceeds the Int literal carrier",
                        )
                    })?;
                    let fixed_capacity =
                        JsNumericCarrier::int_literal(fixed_capacity, self.config.numeric_profile)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "JS fixed collection capacity exceeds the Int numeric profile",
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
                self.lower_map_literal(expression.ty, entries)
            }

            HirExpressionKind::Range { start, end } => {
                let start = self.lower_expr(start)?;
                let end = self.lower_expr(end)?;
                Ok(format!("{{ start: {start}, end: {end} }}"))
            }

            HirExpressionKind::TupleConstruct { elements } => {
                if elements.is_empty() {
                    Ok("undefined".to_owned())
                } else {
                    let lowered = elements
                        .iter()
                        .map(|element| self.lower_expr(element))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(format!("[{}]", lowered.join(", ")))
                }
            }

            HirExpressionKind::TupleGet { tuple, index } => {
                let tuple = self.lower_expr(tuple)?;
                Ok(format!("({tuple})[{index}]"))
            }

            HirExpressionKind::FallibleUnwrapSuccess { result } => {
                let lowered_result = self.lower_expr(result)?;
                Ok(format!("(({lowered_result}).value)"))
            }

            HirExpressionKind::FallibleUnwrapError { result } => {
                let lowered_result = self.lower_expr(result)?;
                Ok(format!("(({lowered_result}).value)"))
            }

            HirExpressionKind::Cast { source, policy } => {
                let lowered_source = self.lower_expr(source)?;
                js_cast_expression_for_policy(*policy, &lowered_source, self.config.numeric_profile)
            }

            HirExpressionKind::VariantPayloadGet {
                carrier,
                source,
                variant_index,
                field_index,
            } => self.lower_variant_payload_get(carrier, source, *variant_index, *field_index),
        }
    }

    fn lower_structural_string(
        &self,
        pieces: &[ConstStringPiece],
    ) -> Result<String, CompilerError> {
        let Some(url_map) = self.config.structural_string_urls.as_ref() else {
            return Err(CompilerError::compiler_error(
                "JavaScript lowering received a structural string without a builder URL map",
            ));
        };

        let mut rendered = String::new();
        for piece in pieces {
            match piece {
                ConstStringPiece::Text(text) => rendered.push_str(self.string_table.resolve(*text)),
                ConstStringPiece::Resource(resource_id) => {
                    let Some(url) = url_map.resource_urls.get(resource_id) else {
                        return Err(CompilerError::compiler_error(format!(
                            "JavaScript lowering has no rendered URL for structural resource {resource_id:?}"
                        )));
                    };
                    rendered.push_str(url);
                }
                ConstStringPiece::SiteRoot => {
                    let Some(url) = url_map.site_root_url.as_deref() else {
                        return Err(CompilerError::compiler_error(
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
    /// WHY: the JS backend uses runtime helpers for field and index access to support the
    /// reactive binding model.
    pub(crate) fn lower_place(&mut self, place: &HirPlace) -> Result<String, CompilerError> {
        match place {
            HirPlace::Local(local_id) => Ok(self.local_name(*local_id)?.to_owned()),

            HirPlace::Field { base, field } => {
                let base = self.lower_place(base)?;
                let field = escape_js_string(self.field_name(*field)?);
                Ok(format!("__moth_field({base}, {field})"))
            }

            HirPlace::Index { base, index } => {
                let base = self.lower_place(base)?;
                let index = self.lower_expr(index)?;
                Ok(format!("__moth_index({base}, {index})"))
            }
        }
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
        fields: &[crate::compiler_frontend::hir::expressions::HirVariantField],
    ) -> Result<String, CompilerError> {
        let mut entries = vec![];
        for field in fields {
            let js_value = self.lower_expr(&field.value)?;
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
            #[cfg(test)]
            HirVariantCarrier::Fallible => {
                let tag = if variant_index == 0 { "ok" } else { "err" };
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
        source: &HirExpression,
        variant_index: usize,
        field_index: usize,
    ) -> Result<String, CompilerError> {
        let source_js = self.lower_expr(source)?;
        let field_name_js = match carrier {
            HirVariantCarrier::Choice { choice_id } => {
                let choice = self.hir.choices.get(choice_id.0 as usize).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JavaScript backend: invalid ChoiceId {choice_id:?} in VariantPayloadGet"
                    ))
                })?;
                let variant = choice.variants.get(variant_index).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JavaScript backend: invalid variant index {variant_index} for choice {choice_id:?}"
                    ))
                })?;
                let field = variant.fields.get(field_index).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JavaScript backend: invalid field index {field_index} for variant {variant_index} of choice {choice_id:?}"
                    ))
                })?;
                escape_js_string(self.string_table.resolve(field.name))
            }
            HirVariantCarrier::Option => "\"value\"".to_owned(),
            #[cfg(test)]
            HirVariantCarrier::Fallible => "\"value\"".to_owned(),
        };
        Ok(format!("({source_js})[{field_name_js}]"))
    }

    pub(crate) fn is_unit_expression(&self, expression: &HirExpression) -> bool {
        if matches!(
            expression.kind,
            HirExpressionKind::TupleConstruct { ref elements } if elements.is_empty()
        ) {
            return true;
        }

        expression.ty == self.type_environment.builtins().none
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
        left: &HirExpression,
        operator: HirBinOp,
        right: &HirExpression,
    ) -> Result<String, CompilerError> {
        let left_type = left.ty;
        let right_type = right.ty;
        let option_equality = if matches!(operator, HirBinOp::Eq | HirBinOp::Ne) {
            self.option_equality_sides(left, right)
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
                _ => unreachable!(),
            };
        }

        // Relational Number/BigInt operators compare exact integer values directly without
        // coercing either carrier to the other. StringAppend remains ordinary JS concatenation.
        let js_operator = match operator {
            HirBinOp::StringAppend => "+",
            HirBinOp::Eq | HirBinOp::Ne => unreachable!("equality returned above"),
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
        left: &HirExpression,
        right: &HirExpression,
    ) -> Option<(OptionComparisonSide, OptionComparisonSide)> {
        let left_side = self.classify_option_comparison_side(left.ty);
        let right_side = self.classify_option_comparison_side(right.ty);

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
    ) -> Result<String, CompilerError> {
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

            _ => {
                return Err(CompilerError::compiler_error(
                    "JavaScript backend received option equality without an option operand",
                ));
            }
        };

        match operator {
            HirBinOp::Eq => Ok(equality),
            HirBinOp::Ne => Ok(format!("(!{equality})")),
            _ => Err(CompilerError::compiler_error(
                "JavaScript backend received non-equality option comparison",
            )),
        }
    }

    fn numeric_type_uses_bigint_carrier(&self, type_id: TypeId) -> Option<bool> {
        if self.type_environment.fixed_scalar(type_id) == Some(FixedScalar::Byte) {
            return Some(false);
        }

        let scalar = NumericScalar::from_type_id(type_id, self.type_environment)?;
        let carrier = JsNumericCarrier::for_scalar(scalar, self.config.numeric_profile)?;
        Some(matches!(carrier, JsNumericCarrier::BigInteger { .. }))
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
        if left_type == self.type_environment.builtins().string
            && right_type == self.type_environment.builtins().string
        {
            return format!("__moth_string_equal({left}, {right})");
        }

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

    /// Lower a unary operator expression to JS.
    ///
    /// WHAT: maps `HirUnaryOp::Neg` to `-` and `HirUnaryOp::Not` to `!`, wrapping the
    /// operand in parentheses for correct precedence.
    /// WHY: JS unary operators have the same semantics as Moth for these cases.
    fn lower_unary_op(
        &mut self,
        operator: HirUnaryOp,
        operand: &HirExpression,
    ) -> Result<String, CompilerError> {
        let operand = self.lower_expr(operand)?;
        let js_operator = match operator {
            HirUnaryOp::Neg => "-",
            HirUnaryOp::Not => "!",
        };

        Ok(format!("({js_operator}{operand})"))
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
        entries: &[HirMapEntry],
    ) -> Result<String, CompilerError> {
        let Some(_map_shape) = self.type_environment.map_shape(type_id) else {
            return Err(CompilerError::compiler_error(
                "JS backend lowered a map literal whose type is not a map",
            ));
        };

        let mut lowered_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let key = self.lower_expr(&entry.key)?;
            let value = self.lower_expr(&entry.value)?;
            lowered_entries.push(format!("[{key}, {value}]"));
        }

        Ok(format!("__moth_map_new([{}])", lowered_entries.join(", ")))
    }
    // -----------------------------
    //  Reactive template lowering
    // -----------------------------

    /// Lower a reactive template value to the backend-owned template-string runtime representation.
    ///
    /// WHAT: returns `__moth_template_string(() => snapshot, __moth_template_collect_dependencies(...))`
    /// carrying a snapshot function and the transitive reactive source dependencies.
    /// WHY: template-string values must preserve dependency metadata for Phase 7 mounting and
    /// rerendering while still snapshotting to plain strings in ordinary string contexts.
    pub(crate) fn lower_reactive_template_value(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        let Some(template) = self
            .hir
            .side_table
            .reactive_template_for_value(expression.id)
        else {
            return self.lower_expr(expression);
        };

        let snapshot_body = self.lower_reactive_template_snapshot_body(expression)?;
        let direct_dependencies = self.lower_reactive_template_direct_dependencies(template);
        let nested_values = self.lower_reactive_template_nested_values(template)?;

        Ok(format!(
            "__moth_template_string(() => {snapshot_body}, __moth_template_collect_dependencies({direct_dependencies}, {nested_values}))"
        ))
    }

    /// Lower the snapshot body of a reactive template value.
    ///
    /// WHAT: for the common accumulator-local result produced by HIR template lowering, the
    /// snapshot re-reads the current local value. Other shapes fall back to ordinary expression
    /// lowering without the reactive wrapper.
    /// WHY: the snapshot function must produce a plain string each time it is called.
    fn lower_reactive_template_snapshot_body(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, CompilerError> {
        match &expression.kind {
            HirExpressionKind::Load(place) => {
                let place_js = self.lower_place(place)?;
                Ok(format!("__moth_template_snapshot(__moth_read({place_js}))"))
            }

            HirExpressionKind::Copy(place) => {
                let place_js = self.lower_place(place)?;
                Ok(format!("__moth_template_snapshot(__moth_read({place_js}))"))
            }

            _ => self.lower_expr_without_reactive_snapshot(expression),
        }
    }

    fn lower_reactive_template_direct_dependencies(
        &self,
        template: &crate::compiler_frontend::hir::reactivity::HirReactiveTemplate,
    ) -> String {
        if template.dependencies.is_empty() {
            return "[]".to_owned();
        }

        let ids = template
            .dependencies
            .iter()
            .map(|dependency| dependency.source.0.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        format!("[{ids}]")
    }

    fn lower_reactive_template_nested_values(
        &self,
        template: &crate::compiler_frontend::hir::reactivity::HirReactiveTemplate,
    ) -> Result<String, CompilerError> {
        if template.template_value_parameters.is_empty() {
            return Ok("[]".to_owned());
        }

        let mut values = Vec::with_capacity(template.template_value_parameters.len());
        for dependency in &template.template_value_parameters {
            let local_name = self.local_name(dependency.parameter)?;
            values.push(format!("__moth_read({local_name})"));
        }

        Ok(format!("[{}]", values.join(", ")))
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
    numeric_profile: crate::compiler_frontend::datatypes::numeric_profile::NumericProfile,
) -> Result<String, CompilerError> {
    match policy {
        BuiltinCastPolicyId::NumericConversion { source, target } => {
            JsNumericConversion::classify(source, target, numeric_profile)
                .map(|conversion| conversion.expression(value, source, target))
        }
        BuiltinCastPolicyId::ByteToU8 | BuiltinCastPolicyId::U8ToByte => Ok(value.to_owned()),
        BuiltinCastPolicyId::NumericToString(scalar) if scalar.is_integer() => {
            Ok(format!("__moth_cast_int_to_string({value})"))
        }
        BuiltinCastPolicyId::NumericToString(scalar) => {
            let precision = scalar
                .binary_float_precision(numeric_profile)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JS backend received a non-numeric string formatting policy for {:?}",
                        scalar
                    ))
                })?;
            Ok(format!(
                "__moth_cast_float_to_string({value}, {}, {:?})",
                binary_float_precision_bits(precision),
                scalar.name()
            ))
        }
        BuiltinCastPolicyId::BoolToString => Ok(format!("__moth_cast_bool_to_string({value})")),
        BuiltinCastPolicyId::CharToString => Ok(format!("__moth_cast_char_to_string({value})")),
        BuiltinCastPolicyId::CharToInt => Ok(format!("__moth_cast_char_to_int({value})")),
        BuiltinCastPolicyId::StringToError => Ok(format!("__moth_cast_string_to_error({value})")),
        BuiltinCastPolicyId::ErrorToString => Ok(format!("__moth_cast_error_to_string({value})")),
        BuiltinCastPolicyId::IntToChar => Ok(format!("__moth_cast_int_to_char({value})")),
        BuiltinCastPolicyId::StringToNumeric(scalar) if scalar.is_integer() => {
            let carrier =
                JsNumericCarrier::for_scalar(scalar, numeric_profile).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JS backend has no integer carrier for {scalar:?}"
                    ))
                })?;
            let (minimum, maximum) = carrier.integer_bounds_js().ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "JS backend has no integer bounds for {scalar:?}"
                ))
            })?;
            Ok(format!(
                "__moth_cast_int({value}, {minimum}, {maximum}, {:?})",
                scalar.name()
            ))
        }
        BuiltinCastPolicyId::StringToNumeric(scalar) => {
            let precision = scalar
                .binary_float_precision(numeric_profile)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JS backend received a non-numeric parse policy for {scalar:?}"
                    ))
                })?;
            Ok(format!(
                "__moth_cast_float({value}, {}, {:?})",
                binary_float_precision_bits(precision),
                scalar.name()
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

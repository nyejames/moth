//! HIR expression, pattern, and place validation for the HIR validator.
//!
//! WHAT: validates expression-ID graphs, typed side payloads, patterns, variant indexing, and
//! place type resolution after HIR lowering.
//! WHY: expression and place invariants are shared by borrow validation and every backend. Keeping
//! them together makes graph validation flow explicit.

use super::HirValidator;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::comparison_supported;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, ValueKind,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;

/// One step of the explicit expression walk driven by `HirValidator::validate_expression`.
///
/// WHY: `Enter` checks a row and queues its children. `Exit` runs the checks that need those
/// children validated, then clears the row's active mark. A work list bounds validation depth by
/// heap memory rather than the native stack.
pub(super) enum ExpressionFrame {
    Enter(HirValueId),
    Exit(HirValueId),
}

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Expression & Pattern Validation
    // -------------------------

    pub(super) fn expression_row(
        &self,
        expression_id: HirValueId,
        anchor: Option<HirLocation>,
    ) -> Result<&crate::compiler_frontend::hir::expressions::HirExpression, CompilerError> {
        self.module
            .expressions
            .get_expression(expression_id)
            .ok_or_else(|| {
                self.error_with_hir(format!("Invalid expression id {expression_id:?}"), anchor)
            })
    }

    pub(super) fn value_ids(
        &self,
        range: crate::compiler_frontend::hir::expression_store::HirValueRange,
        anchor: Option<HirLocation>,
    ) -> Result<&[HirValueId], CompilerError> {
        self.module
            .expressions
            .get_values(range)
            .ok_or_else(|| self.error_with_hir("Invalid HIR expression-value range", anchor))
    }

    pub(super) fn struct_fields(
        &self,
        range: crate::compiler_frontend::hir::expression_store::HirStructFieldRange,
        anchor: Option<HirLocation>,
    ) -> Result<&[(crate::compiler_frontend::hir::ids::FieldId, HirValueId)], CompilerError> {
        self.module
            .expressions
            .get_struct_fields(range)
            .ok_or_else(|| self.error_with_hir("Invalid HIR struct-field range", anchor))
    }

    pub(super) fn variant_fields(
        &self,
        range: crate::compiler_frontend::hir::expression_store::HirVariantFieldRange,
        anchor: Option<HirLocation>,
    ) -> Result<&[crate::compiler_frontend::hir::expressions::HirVariantField], CompilerError> {
        self.module
            .expressions
            .get_variant_fields(range)
            .ok_or_else(|| self.error_with_hir("Invalid HIR variant-field range", anchor))
    }

    pub(super) fn map_entries(
        &self,
        range: crate::compiler_frontend::hir::expression_store::HirMapEntryRange,
        anchor: Option<HirLocation>,
    ) -> Result<&[crate::compiler_frontend::hir::expressions::HirMapEntry], CompilerError> {
        self.module
            .expressions
            .get_map_entries(range)
            .ok_or_else(|| self.error_with_hir("Invalid HIR map-entry range", anchor))
    }

    pub(super) fn string_pieces(
        &self,
        range: crate::compiler_frontend::hir::expression_store::HirStringPieceRange,
        anchor: Option<HirLocation>,
    ) -> Result<
        &[crate::compiler_frontend::ast::const_values::store::ConstStringPiece],
        CompilerError,
    > {
        self.module
            .expressions
            .get_string_pieces(range)
            .ok_or_else(|| self.error_with_hir("Invalid HIR structural-string piece range", anchor))
    }

    pub(super) fn projections(
        &self,
        range: crate::compiler_frontend::hir::expression_store::HirProjectionRange,
        anchor: Option<HirLocation>,
    ) -> Result<&[HirProjection], CompilerError> {
        self.module
            .expressions
            .get_projections(range)
            .ok_or_else(|| self.error_with_hir("Invalid HIR place-projection range", anchor))
    }

    pub(super) fn validate_match_arm(
        &self,
        source_block_id: BlockId,
        arm: &HirMatchArm,
        pattern_subject_type_id: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.validate_pattern(&arm.pattern, pattern_subject_type_id, anchor)?;

        if let Some(guard) = arm.guard {
            self.validate_expression(guard, anchor)?;
        }

        self.require_block_id(arm.body, anchor)?;
        self.require_same_function_cfg_owner(source_block_id, arm.body, anchor)?;
        Ok(())
    }

    pub(super) fn validate_pattern(
        &self,
        pattern: &HirPattern,
        pattern_subject_type_id: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        match pattern {
            HirPattern::Literal(value) => {
                self.validate_literal_pattern_expression(*value, pattern_subject_type_id, anchor)?;
            }

            HirPattern::OptionNone => {}

            HirPattern::OptionValue { value } => {
                self.validate_literal_pattern_expression(*value, pattern_subject_type_id, anchor)?;
            }

            HirPattern::OptionRelational { value, .. } => {
                self.validate_literal_pattern_expression(*value, pattern_subject_type_id, anchor)?;
                self.validate_relational_pattern_expression(*value, anchor)?;
            }

            HirPattern::Wildcard => {}

            HirPattern::Relational { value, .. } => {
                self.validate_literal_pattern_expression(*value, pattern_subject_type_id, anchor)?;
                self.validate_relational_pattern_expression(*value, anchor)?;
            }

            HirPattern::ChoiceVariant { choice_id, .. } => {
                if self.module.choices.get(choice_id.0 as usize).is_none() {
                    return Err(self.error_with_hir(
                        format!("Invalid ChoiceId {choice_id:?} in pattern"),
                        anchor,
                    ));
                }
            }

            HirPattern::OptionPresent => {
                // Present-capture patterns have no embedded expression to validate.
            }
        }

        Ok(())
    }

    /// WHAT: checks that a relational pattern's inner expression is an ordered literal
    ///       (int, float, fixed scalar, Dec, or char).
    /// WHY: relational patterns compare against compile-time constants; only ordered literal
    ///      kinds are valid comparison targets.
    pub(super) fn validate_relational_pattern_expression(
        &self,
        expression_id: HirValueId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let expression = self.expression_row(expression_id, anchor)?;
        if !matches!(
            expression.kind,
            HirExpressionKind::FixedScalar(_)
                | HirExpressionKind::Number(_)
                | HirExpressionKind::Char(_)
        ) {
            return Err(self.error_with_hir(
                "Match relational pattern must be int/float/fixed-scalar/dec/char",
                anchor,
            ));
        }

        Ok(())
    }

    /// WHAT: checks that a literal pattern's inner expression is const-valued
    ///       and is one of the allowed literal kinds (including fixed scalars and Dec values).
    /// WHY: match arms destructure against compile-time constants; non-const or
    ///      structurally complex expressions are not valid pattern targets.
    pub(super) fn validate_literal_pattern_expression(
        &self,
        expression_id: HirValueId,
        pattern_subject_type_id: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.validate_expression(expression_id, anchor)?;
        let expression = self.expression_row(expression_id, anchor)?;

        if expression.value_kind != ValueKind::Const {
            return Err(
                self.error_with_hir("Match literal pattern must have ValueKind::Const", anchor)
            );
        }

        if !matches!(
            expression.kind,
            HirExpressionKind::FixedScalar(_)
                | HirExpressionKind::Number(_)
                | HirExpressionKind::Bool(_)
                | HirExpressionKind::Char(_)
                | HirExpressionKind::StringLiteral(_)
        ) {
            return Err(self.error_with_hir(
                "Match literal pattern must be int/float/fixed-scalar/number/bool/char/string",
                anchor,
            ));
        }
        if expression.ty != pattern_subject_type_id {
            return Err(self.error_with_hir(
                "Match pattern value type does not match the direct scrutinee type",
                anchor,
            ));
        }

        if let HirExpressionKind::FixedScalar(value) = &expression.kind
            && self.type_environment.fixed_scalar(pattern_subject_type_id) != Some(value.scalar())
        {
            return Err(self.error_with_hir(
                "HIR fixed-scalar pattern value does not match its direct scrutinee type",
                anchor,
            ));
        }

        Ok(())
    }

    /// WHAT: validates an expression row and every row reachable through its typed store edges.
    /// WHY: an explicit work list replaces recursion. Active marks still reject cycles, and error
    /// paths clear them so a retry reports the same diagnostic.
    pub(super) fn validate_expression(
        &self,
        expression_id: HirValueId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        // Taking the buffer leaves an empty Vec behind. Putting it back keeps its capacity, so
        // leaf and single-child rows allocate nothing after the first walk.
        let mut frames = self.expression_frames.take();
        frames.push(ExpressionFrame::Enter(expression_id));
        let result = self.run_expression_frames(&mut frames, anchor);

        // Exit frames still queued name rows this walk marked and has not left. Enter frames
        // were never marked, so they need no reset.
        for frame in frames.drain(..) {
            if let ExpressionFrame::Exit(row) = frame {
                self.active_expression_rows[row.0 as usize].set(false);
            }
        }
        self.expression_frames.set(frames);
        result
    }

    /// Drains the work list until it is empty or a check fails.
    fn run_expression_frames(
        &self,
        frames: &mut Vec<ExpressionFrame>,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        while let Some(frame) = frames.pop() {
            match frame {
                ExpressionFrame::Enter(expression_id) => {
                    self.enter_expression(expression_id, anchor, frames)?;
                }
                ExpressionFrame::Exit(expression_id) => {
                    self.exit_expression(expression_id, anchor)?;
                }
            }
        }
        Ok(())
    }

    /// Checks one row before its children, then queues the row's exit and its children.
    fn enter_expression(
        &self,
        expression_id: HirValueId,
        anchor: Option<HirLocation>,
        frames: &mut Vec<ExpressionFrame>,
    ) -> Result<(), CompilerError> {
        let expression = self.expression_row(expression_id, anchor)?;
        let expression_index = expression_id.0 as usize;
        if self.active_expression_rows[expression_index].replace(true) {
            return Err(self.error_with_hir(
                format!("HIR expression graph contains a cycle at {expression_id:?}"),
                anchor,
            ));
        }
        // Queued before any fallible check, so an error in this row still clears its mark.
        frames.push(ExpressionFrame::Exit(expression_id));
        self.validate_expression_contents(expression_id, expression, anchor)?;

        match &expression.kind {
            // Index children pop outermost first, matching the former recursive place walk.
            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
                for index in self.module.expressions.place_index_values(*place) {
                    frames.push(ExpressionFrame::Enter(index));
                }
            }
            _ => {
                if let Some(first_child) =
                    self.module.expressions.push_child_values_in_source_order(
                        expression_id,
                        frames,
                        ExpressionFrame::Enter,
                    )
                {
                    frames.push(first_child);
                }
            }
        }
        Ok(())
    }

    /// Clears one row's active mark, then runs the checks that read its validated children.
    fn exit_expression(
        &self,
        expression_id: HirValueId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        // The exit frame is already popped, so an error below would no longer unwind this mark.
        // Clear it first.
        self.active_expression_rows[expression_id.0 as usize].set(false);
        let expression = self.expression_row(expression_id, anchor)?;
        self.validate_expression_after_children(expression, anchor)
    }

    /// Runs the checks for one row that precede its children.
    ///
    /// WHY: a row's own diagnostics come before any child's, as in the earlier recursive walk.
    /// Checks that need validated children live in `validate_expression_after_children`.
    fn validate_expression_contents(
        &self,
        expression_id: HirValueId,
        expression: &HirExpression,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        // A value's source span lives on its row; the side table maps only AST spans.
        let value_location = HirLocation::Value(expression_id);
        if expression.span.is_some()
            && self
                .module
                .side_table
                .ast_span_for_hir(value_location)
                .is_none()
        {
            return Err(self.error_with_hir(
                format!(
                    "Value {} is missing AST->HIR side-table mapping",
                    expression_id
                ),
                anchor,
            ));
        }

        self.require_type_id(expression.ty, anchor)?;
        self.require_region_id(expression.region, anchor)?;

        if expression.value_kind == ValueKind::Place
            && !matches!(&expression.kind, HirExpressionKind::Load(_))
        {
            return Err(self.error_with_hir(
                "ValueKind::Place is only valid for a direct HirExpressionKind::Load",
                anchor,
            ));
        }

        match &expression.kind {
            HirExpressionKind::Number(value) => {
                if self.type_environment.number_scale(expression.ty) != Some(value.scale()) {
                    return Err(self.error_with_hir(
                        format!(
                            "HIR Dec literal scale {} does not match expression type {:?}",
                            value.scale(),
                            expression.ty
                        ),
                        anchor,
                    ));
                }
            }

            // Leaf literals carry no sub-expressions; no further validation needed. The fixed
            // scalar carrier guarantees finite binary floats and exact-bit payloads at
            // construction, so HIR never consumes an invalid numeric value.
            HirExpressionKind::FixedScalar(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => {}

            HirExpressionKind::StructuralString { pieces } => {
                let _ = self.string_pieces(*pieces, anchor)?;
            }

            HirExpressionKind::VariantConstruct {
                carrier,
                variant_index,
                fields: field_range,
            } => {
                let fields = self.variant_fields(*field_range, anchor)?;
                // Validate variant-index bounds and field arity per carrier kind.
                match carrier {
                    HirVariantCarrier::Choice { choice_id } => {
                        let Some(choice) = self.module.choices.get(choice_id.0 as usize) else {
                            return Err(self.error_with_hir(
                                format!("Invalid ChoiceId {choice_id:?} in VariantConstruct"),
                                anchor,
                            ));
                        };
                        if *variant_index >= choice.variants.len() {
                            return Err(self.error_with_hir(
                                format!(
                                    "Variant index {variant_index} out of range for choice {choice_id:?} with {} variants",
                                    choice.variants.len()
                                ),
                                anchor,
                            ));
                        }
                        let variant = &choice.variants[*variant_index];
                        if fields.len() != variant.fields.len() {
                            return Err(self.error_with_hir(
                                format!(
                                    "VariantConstruct field count {} does not match choice variant field count {}",
                                    fields.len(),
                                    variant.fields.len()
                                ),
                                anchor,
                            ));
                        }
                        for (actual, expected) in fields.iter().zip(variant.fields.iter()) {
                            if actual.name != Some(expected.name) {
                                return Err(self.error_with_hir(
                                    format!(
                                        "VariantConstruct field name {:?} does not match declared name {:?}",
                                        actual.name, expected.name
                                    ),
                                    anchor,
                                ));
                            }
                            let actual_value = self.expression_row(actual.value, anchor)?;
                            if actual_value.ty != expected.ty {
                                return Err(self.error_with_hir(
                                    format!(
                                        "VariantConstruct field type mismatch for field {:?}",
                                        expected.name
                                    ),
                                    anchor,
                                ));
                            }
                        }
                    }
                    HirVariantCarrier::Option => {
                        if *variant_index > 1 {
                            return Err(self.error_with_hir(
                                format!(
                                    "VariantConstruct(Option) variant index {variant_index} out of range (valid: 0..=1)"
                                ),
                                anchor,
                            ));
                        }
                        let expected = if *variant_index == 0 { 0 } else { 1 };
                        if fields.len() != expected {
                            return Err(self.error_with_hir(
                                format!(
                                    "VariantConstruct(Option) field count {} does not match expected {} for variant index {}",
                                    fields.len(), expected, variant_index
                                ),
                                anchor,
                            ));
                        }
                        if *variant_index == 1 {
                            let Some(inner_type) =
                                self.type_environment.option_inner_type(expression.ty)
                            else {
                                return Err(self.error_with_hir(
                                    "VariantConstruct(Option) expression type is not an option",
                                    anchor,
                                ));
                            };
                            let some_value = self.expression_row(fields[0].value, anchor)?;
                            if some_value.ty != inner_type {
                                return Err(self.error_with_hir(
                                    "VariantConstruct(Option) some-value type does not match option inner type",
                                    anchor,
                                ));
                            }
                        }
                    }
                }
            }

            HirExpressionKind::RouteConstruct {
                choice_id,
                variant_index,
                fields: field_range,
                future_input,
            } => {
                let fields = self.variant_fields(*field_range, anchor)?;
                let Some(choice) = self.module.choices.get(choice_id.0 as usize) else {
                    return Err(self.error_with_hir(
                        format!("Invalid ChoiceId {choice_id:?} in RouteConstruct"),
                        anchor,
                    ));
                };
                if choice.id != *choice_id || choice.frontend_type_id != expression.ty {
                    return Err(self.error_with_hir(
                        "RouteConstruct choice identity does not match its expression type",
                        anchor,
                    ));
                }
                let Some(variant) = choice.variants.get(*variant_index) else {
                    return Err(self.error_with_hir(
                        format!(
                            "RouteConstruct variant index {variant_index} is out of range for choice {choice_id:?}"
                        ),
                        anchor,
                    ));
                };
                // One future input supplies exactly the trailing field, so captures fill every
                // other field; without an input they fill the whole payload.
                if fields.len() + usize::from(*future_input) != variant.fields.len() {
                    return Err(self.error_with_hir(
                        "RouteConstruct captures must fill every variant field except the one future input",
                        anchor,
                    ));
                }
                for (actual, expected) in fields.iter().zip(variant.fields.iter()) {
                    if actual.name != Some(expected.name) {
                        return Err(self.error_with_hir(
                            format!(
                                "RouteConstruct field name {:?} does not match declared prefix name {:?}",
                                actual.name, expected.name
                            ),
                            anchor,
                        ));
                    }
                    let actual_value = self.expression_row(actual.value, anchor)?;
                    if actual_value.ty != expected.ty {
                        return Err(self.error_with_hir(
                            format!(
                                "RouteConstruct field type mismatch for prefix field {:?}",
                                expected.name
                            ),
                            anchor,
                        ));
                    }
                }
            }
            // Reads the root and projection list before the walk queues index children, so a
            // malformed place fails here. The type resolves after the indexes validate.
            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
                self.require_local_for_anchor(place.root, anchor)?;
                let _ = self.projections(place.projections, anchor)?;
            }

            HirExpressionKind::BinOp { op, left, right } => {
                let is_comparison = matches!(
                    op,
                    HirBinOp::Eq
                        | HirBinOp::Ne
                        | HirBinOp::Lt
                        | HirBinOp::Le
                        | HirBinOp::Gt
                        | HirBinOp::Ge
                );
                if is_comparison {
                    let left_expression = self.expression_row(*left, anchor)?;
                    let right_expression = self.expression_row(*right, anchor)?;
                    let left_scalar =
                        NumericScalar::from_type_id(left_expression.ty, self.type_environment);
                    let right_scalar =
                        NumericScalar::from_type_id(right_expression.ty, self.type_environment);
                    let number_involved = matches!(left_scalar, Some(NumericScalar::Number(_)))
                        || matches!(right_scalar, Some(NumericScalar::Number(_)));

                    if number_involved
                        && !matches!(
                            (left_scalar, right_scalar),
                            (
                                Some(NumericScalar::Number(_)),
                                Some(NumericScalar::Number(_))
                            )
                        )
                    {
                        return Err(self.error_with_hir(
                            "Dec comparison integer operands must be explicitly converted to the same Dec scale",
                            anchor,
                        ));
                    }

                    if let (Some(left_scalar), Some(right_scalar)) = (left_scalar, right_scalar)
                        && !comparison_supported(left_scalar, right_scalar)
                    {
                        return Err(self.error_with_hir(
                            "HirBinOp comparison operands have incompatible types",
                            anchor,
                        ));
                    }

                    if expression.ty != self.type_environment.builtins().bool {
                        return Err(
                            self.error_with_hir("HirBinOp comparison must produce Bool", anchor)
                        );
                    }
                }

                if *op == HirBinOp::StringAppend {
                    let string_type = self.type_environment.builtins().string;
                    if expression.ty != string_type
                        || self.expression_row(*left, anchor)?.ty != string_type
                    {
                        return Err(self.error_with_hir(
                            "HirBinOp::StringAppend must produce a String from a String accumulator",
                            anchor,
                        ));
                    }
                }
            }

            HirExpressionKind::UnaryOp { op, .. } => {
                if *op == HirUnaryOp::Neg {
                    return Err(self.error_with_hir(
                        "Plain HirUnaryOp::Neg must be lowered through HirStatementKind::NumericOp",
                        anchor,
                    ));
                }
            }

            HirExpressionKind::StructConstruct {
                struct_id,
                fields: field_range,
            } => {
                // Validate struct existence and field ownership, then field sub-expressions.
                self.require_struct_id(*struct_id, anchor)?;
                for (field_id, _) in self.struct_fields(*field_range, anchor)? {
                    self.require_field_owned_by(*field_id, *struct_id, anchor)?;
                }
            }

            // Collection literals: validate type is a collection and validate each element.
            HirExpressionKind::Collection(element_range) => {
                if self
                    .type_environment
                    .collection_shape(expression.ty)
                    .is_none()
                {
                    return Err(self.error_with_hir(
                        "HirExpressionKind::Collection expression type is not a collection type",
                        anchor,
                    ));
                }
                // The checked range read reports a malformed range before the listing reads it.
                self.value_ids(*element_range, anchor)?;
            }

            // Map literals: validate type is a map and validate key/value sub-expressions.
            HirExpressionKind::MapLiteral(entry_range) => {
                if self.type_environment.map_shape(expression.ty).is_none() {
                    return Err(self.error_with_hir(
                        "HirExpressionKind::MapLiteral expression type is not a map type",
                        anchor,
                    ));
                }
                // The checked range read reports a malformed range before the listing reads it.
                self.map_entries(*entry_range, anchor)?;
            }

            // Payload and cast checks read their validated source, so they run after the walk.
            HirExpressionKind::Range { .. }
            | HirExpressionKind::VariantPayloadGet { .. }
            | HirExpressionKind::Cast { .. } => {}
        }
        Ok(())
    }

    /// Runs the checks that read a row's validated children.
    ///
    /// WHY: a place's type resolves from its validated indexes, and a payload read or cast reads
    /// its source row, so these checks run only after the walk has validated those children.
    fn validate_expression_after_children(
        &self,
        expression: &HirExpression,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        match &expression.kind {
            HirExpressionKind::Load(place) => {
                let place_type = self.resolve_place_type(place, anchor)?;
                if expression.ty != place_type {
                    return Err(self.error_with_hir(
                        "Load expression type does not match its place type",
                        anchor,
                    ));
                }
            }

            HirExpressionKind::Copy(place) => {
                let place_type = self.resolve_place_type(place, anchor)?;
                if expression.ty != place_type {
                    return Err(self.error_with_hir(
                        "Copy expression type does not match its place type",
                        anchor,
                    ));
                }
            }

            // Variant payload extraction: check carrier bounds once the source has validated.
            HirExpressionKind::VariantPayloadGet {
                carrier,
                variant_index,
                field_index,
                ..
            } => match carrier {
                HirVariantCarrier::Choice { choice_id } => {
                    let Some(choice) = self.module.choices.get(choice_id.0 as usize) else {
                        return Err(self.error_with_hir(
                            format!("Invalid ChoiceId {choice_id:?} in VariantPayloadGet"),
                            anchor,
                        ));
                    };
                    if *variant_index >= choice.variants.len() {
                        return Err(self.error_with_hir(
                                format!(
                                    "VariantPayloadGet variant index {variant_index} out of range for choice {choice_id:?} with {} variants",
                                    choice.variants.len()
                                ),
                                anchor,
                            ));
                    }
                    let variant = &choice.variants[*variant_index];
                    if *field_index >= variant.fields.len() {
                        return Err(self.error_with_hir(
                                format!(
                                    "VariantPayloadGet field index {field_index} out of range for variant with {} fields",
                                    variant.fields.len()
                                ),
                                anchor,
                            ));
                    }
                }
                HirVariantCarrier::Option => {
                    if *variant_index > 1 {
                        return Err(self.error_with_hir(
                                format!(
                                    "VariantPayloadGet variant index {variant_index} out of range for Option (valid: 0..=1)",
                                ),
                                anchor,
                            ));
                    }
                    let max_fields = if *variant_index == 0 { 0 } else { 1 };
                    if *field_index >= max_fields {
                        return Err(self.error_with_hir(
                                format!(
                                    "VariantPayloadGet field index {field_index} out of range for Option variant {variant_index} with {max_fields} fields",
                                ),
                                anchor,
                            ));
                    }
                }
            },

            HirExpressionKind::Cast { source, policy } => {
                if let Some(policy_target_type) =
                    self.validate_numeric_cast_source_type(*policy, *source, anchor)?
                    && expression.ty != policy_target_type
                {
                    return Err(self.error_with_hir(
                        "Cast expression type does not match the policy target type",
                        anchor,
                    ));
                }
                self.validate_number_cast_policy(
                    *policy,
                    BuiltinCastFallibility::Infallible,
                    anchor,
                )?;
            }

            _ => {}
        }

        Ok(())
    }

    // -------------------------
    //  Place Validation
    // -------------------------

    pub(super) fn validate_place(
        &self,
        place: &HirPlace,
        anchor: Option<HirLocation>,
    ) -> Result<TypeId, CompilerError> {
        self.require_local_for_anchor(place.root, anchor)?;
        // Flat projections are stored root-outward. Validate index expressions in reverse to
        // preserve the former outermost-first diagnostic order, then resolve types root-outward.
        let projections = self.projections(place.projections, anchor)?;
        for projection in projections.iter().rev() {
            if let HirProjection::Index(index) = projection {
                self.validate_expression(*index, anchor)?;
            }
        }
        self.resolve_place_type(place, anchor)
    }

    /// Resolves a place's type root-outward from its root local.
    ///
    /// WHY: a Load or Copy validates its index children during the walk, then resolves the type.
    /// Keeping the type chain separate lets the walk and `validate_place` share it.
    fn resolve_place_type(
        &self,
        place: &HirPlace,
        anchor: Option<HirLocation>,
    ) -> Result<TypeId, CompilerError> {
        let projections = self.projections(place.projections, anchor)?;

        let mut current_type = self.local_types.get(&place.root).copied().ok_or_else(|| {
            self.error_with_hir(format!("Unknown local id {:?}", place.root), anchor)
        })?;
        for projection in projections {
            self.require_type_id(current_type, anchor)?;
            current_type = match projection {
                HirProjection::Field(field) => {
                    if !self.is_struct_type(current_type) {
                        return Err(self.error_with_hir(
                            "Field place base does not resolve to struct type",
                            anchor,
                        ));
                    }
                    let owner_struct_id =
                        self.field_owner.get(field).copied().ok_or_else(|| {
                            self.error_with_hir(format!("Unknown field id {field:?}"), anchor)
                        })?;
                    self.require_field_owned_by(*field, owner_struct_id, anchor)?;
                    let owner_type = self
                        .struct_types
                        .get(&owner_struct_id)
                        .copied()
                        .ok_or_else(|| {
                            self.error_with_hir(
                                format!(
                                    "Field {field:?} has no owning struct type in HIR metadata"
                                ),
                                anchor,
                            )
                        })?;
                    if current_type != owner_type {
                        return Err(self.error_with_hir(
                            format!(
                                "Field {field:?} owner type {owner_type:?} does not match place base type {current_type:?}"
                            ),
                            anchor,
                        ));
                    }
                    self.field_types.get(field).copied().ok_or_else(|| {
                        self.error_with_hir(format!("Unknown field id {field:?}"), anchor)
                    })?
                }
                HirProjection::Index(_) => self
                    .type_environment
                    .collection_element_type(current_type)
                    .ok_or_else(|| {
                        self.error_with_hir(
                            "Index place base does not resolve to collection type",
                            anchor,
                        )
                    })?,
            };
        }

        Ok(current_type)
    }

    /// WHAT: returns `true` when `type_id` resolves to a struct type (concrete or generic instance).
    /// WHY: field-place validation needs to distinguish struct types from other compound types.
    pub(super) fn is_struct_type(&self, type_id: TypeId) -> bool {
        match self.type_environment.get(type_id) {
            Some(TypeDefinition::Struct(..)) => true,
            Some(TypeDefinition::GenericInstance(instance)) => self
                .type_environment
                .struct_definition(instance.base)
                .is_some(),
            _ => false,
        }
    }
}

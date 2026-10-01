//! Block, statement, terminator, and side-table mapping validation for HIR.
//!
//! WHAT: walks executable block contents and checks local regions, source mappings, terminators,
//! and contained expression/place references.
//! WHY: the HIR side table is the bridge back to AST and source locations for later analysis and
//! infrastructure errors.

use super::HirValidator;
use crate::compiler_frontend::builtins::casts::evidence::lookup_builtin_evidence;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, negation_domain,
};
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::numeric::{HirNumericOperands, NumericFailureMode};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{
    HirTerminator, classify_assertion_message_evaluation,
};

#[derive(Clone, Copy)]
enum FallibleReturnSlot {
    Success,
    Error,
}

impl FallibleReturnSlot {
    fn terminator_name(self) -> &'static str {
        match self {
            FallibleReturnSlot::Success => "ReturnSuccess",
            FallibleReturnSlot::Error => "ReturnError",
        }
    }

    fn slot_name(self) -> &'static str {
        match self {
            FallibleReturnSlot::Success => "success",
            FallibleReturnSlot::Error => "error",
        }
    }

    fn select_type(self, success_type: TypeId, error_type: TypeId) -> TypeId {
        match self {
            FallibleReturnSlot::Success => success_type,
            FallibleReturnSlot::Error => error_type,
        }
    }
}

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Block & Statement Validation
    // -------------------------

    pub(super) fn validate_blocks(&self) -> Result<(), CompilerError> {
        for block in &self.module.blocks {
            if matches!(block.terminator, HirTerminator::Uninitialized) {
                return Err(self.error_with_hir(
                    format!(
                        "Block {} still has placeholder terminator Uninitialized after HIR lowering",
                        block.id
                    ),
                    Some(HirLocation::Block(block.id)),
                ));
            }

            self.require_region_id(block.region, Some(HirLocation::Block(block.id)))?;

            for local in &block.locals {
                self.require_type_id(local.ty, Some(HirLocation::Local(local.id)))?;
                self.require_region_id(local.region, Some(HirLocation::Local(local.id)))?;
            }

            for statement in &block.statements {
                self.validate_statement_mappings(statement)?;
                self.validate_statement(statement)?;
            }

            self.validate_terminator_mapping(block.id)?;
            self.validate_terminator(block.id, &block.terminator)?;
        }

        Ok(())
    }

    pub(super) fn validate_statement_mappings(
        &self,
        statement: &HirStatement,
    ) -> Result<(), CompilerError> {
        // Compiler-generated statements intentionally have no authored span and therefore no
        // side-table source mapping. Authored statements must carry both reversible provenance
        // and the exact diagnostic span.
        if statement.span.is_none() {
            return Ok(());
        }

        let statement_location = HirLocation::Statement(statement.id);
        if self
            .module
            .side_table
            .ast_span_for_hir(statement_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!(
                    "Statement {} is missing AST->HIR side-table mapping",
                    statement.id
                ),
                Some(statement_location),
            ));
        }

        if self
            .module
            .side_table
            .hir_source_span_for_hir(statement_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!(
                    "Statement {} is missing HIR source side-table mapping",
                    statement.id
                ),
                Some(statement_location),
            ));
        }

        Ok(())
    }

    pub(super) fn validate_terminator_mapping(
        &self,
        block_id: BlockId,
    ) -> Result<(), CompilerError> {
        // Only authored terminators have an exact syntax marker. Generated CFG terminators are
        // deliberately span-free and do not need synthetic provenance.
        if self.module.side_table.terminator_span(block_id).is_none() {
            return Ok(());
        }

        let terminator_location = HirLocation::Terminator(block_id);
        if self
            .module
            .side_table
            .ast_span_for_hir(terminator_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!("Block {block_id} terminator is missing AST->HIR side-table mapping"),
                Some(terminator_location),
            ));
        }

        if self
            .module
            .side_table
            .hir_source_span_for_hir(terminator_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!("Block {block_id} terminator is missing HIR source side-table mapping",),
                Some(terminator_location),
            ));
        }

        Ok(())
    }

    pub(super) fn validate_statement(&self, statement: &HirStatement) -> Result<(), CompilerError> {
        let anchor = Some(HirLocation::Statement(statement.id));
        match &statement.kind {
            HirStatementKind::Assign { target, value } => {
                let _ = self.validate_place(target, anchor)?;
                self.validate_expression(value, anchor)?;
            }

            HirStatementKind::Call { args, result, .. } => {
                for arg in args {
                    self.validate_expression(arg, anchor)?;
                }

                if let Some(local_id) = result {
                    self.require_local_id(*local_id, anchor)?;
                }
            }

            HirStatementKind::Expr(expression) => {
                self.validate_expression(expression, anchor)?;
            }

            HirStatementKind::MapOp {
                receiver,
                args,
                result,
                ..
            } => {
                self.validate_expression(receiver, anchor)?;
                for arg in args {
                    self.validate_expression(arg, anchor)?;
                }
                if let Some(local_id) = result {
                    self.require_local_id(*local_id, anchor)?;
                }
            }

            HirStatementKind::Drop(local) => {
                self.require_local_id(*local, anchor)?;
            }

            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                self.require_local_id(*vec_local, anchor)?;
                self.validate_expression(value, anchor)?;
            }

            HirStatementKind::CastOp {
                policy,
                source,
                result,
            } => {
                self.validate_expression(source, anchor)?;
                let expected_result_type =
                    self.validate_numeric_cast_source_type(*policy, source, anchor)?;
                self.validate_number_cast_policy_fallibility_for_statement(*policy, anchor)?;

                if let Some(local_id) = result {
                    self.require_local_id(*local_id, anchor)?;

                    if let Some(expected_result_type) = expected_result_type {
                        let Some(result_type) = self.local_types.get(local_id).copied() else {
                            return Err(self.error_with_hir(
                                "CastOp result local has no registered type",
                                anchor,
                            ));
                        };
                        self.validate_numeric_cast_result_type(
                            *policy,
                            result_type,
                            expected_result_type,
                            anchor,
                        )?;
                    }
                }
            }

            HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } => {
                if op.is_unary() != matches!(operands, HirNumericOperands::Unary { .. }) {
                    return Err(self.error_with_hir(
                        format!("NumericOp::{op} operand shape does not match the operation arity"),
                        anchor,
                    ));
                }

                // NumericOp operands normally share their result domain. Dec power alone keeps
                // its exponent in the canonical profile Int type.
                let domain_type = self.numeric_scalar_type_id(op.domain, anchor)?;
                let number_power = op.operator == NumericOperator::Power
                    && matches!(op.domain, NumericScalar::Number(_));

                match operands {
                    HirNumericOperands::Unary { operand } => {
                        self.validate_expression(operand, anchor)?;

                        if operand.ty != domain_type {
                            return Err(self.error_with_hir(
                                format!(
                                    "NumericOp::{op} operand type does not match the domain type"
                                ),
                                anchor,
                            ));
                        }
                    }
                    HirNumericOperands::Binary { left, right } => {
                        self.validate_expression(left, anchor)?;
                        self.validate_expression(right, anchor)?;

                        let right_type = if number_power {
                            self.type_environment.builtins().int
                        } else {
                            domain_type
                        };
                        if left.ty != domain_type || right.ty != right_type {
                            return Err(self.error_with_hir(
                                format!(
                                    "NumericOp::{op} operands do not match the required domain types"
                                ),
                                anchor,
                            ));
                        }
                    }
                }

                let operator_domain_valid = match op.operator {
                    NumericOperator::Negate => negation_domain(op.domain) == Some(op.domain),
                    NumericOperator::Power if number_power => {
                        binary_operation_domain(op.operator, op.domain, NumericScalar::Int)
                            == Some(op.domain)
                    }
                    _ => {
                        binary_operation_domain(op.operator, op.domain, op.domain)
                            == Some(op.domain)
                    }
                };

                if !operator_domain_valid {
                    return Err(self.error_with_hir(
                        format!("NumericOp::{op} operator does not match the domain"),
                        anchor,
                    ));
                }

                self.require_local_id(*result, anchor)?;

                let Some(result_type) = self.local_types.get(result).copied() else {
                    return Err(self
                        .error_with_hir("NumericOp result local has no registered type", anchor));
                };

                match failure_mode {
                    NumericFailureMode::Trap => {
                        if result_type != domain_type {
                            return Err(self.error_with_hir(
                                format!(
                                    "NumericOp::{op} Trap result local has the wrong success type"
                                ),
                                anchor,
                            ));
                        }
                    }

                    NumericFailureMode::ReturnError => {
                        let Some((carrier_success_type, _)) =
                            self.type_environment.fallible_carrier_slots(result_type)
                        else {
                            return Err(self.error_with_hir(
                                "NumericOp ReturnError result local must have an internal fallible carrier type",
                                anchor,
                            ));
                        };

                        if carrier_success_type != domain_type {
                            return Err(self.error_with_hir(
                                format!(
                                    "NumericOp::{op} ReturnError carrier has the wrong success type"
                                ),
                                anchor,
                            ));
                        }
                    }
                }
            }

            HirStatementKind::FloatRangeCandidate {
                current,
                step,
                end,
                ascending,
                domain,
                candidate_result,
                in_range_result,
                ..
            } => {
                if !matches!(
                    domain,
                    NumericScalar::Float
                        | NumericScalar::Fixed(FixedScalar::F32 | FixedScalar::F64)
                ) {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate requires a profile Float, F32, or F64 domain",
                        anchor,
                    ));
                }

                self.validate_expression(current, anchor)?;
                self.validate_expression(step, anchor)?;
                self.validate_expression(end, anchor)?;
                self.validate_expression(ascending, anchor)?;

                let domain_type = domain.type_id(self.type_environment);
                if current.ty != domain_type || step.ty != domain_type || end.ty != domain_type {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate operands must match its numeric domain",
                        anchor,
                    ));
                }
                if ascending.ty != self.type_environment.builtins().bool {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate direction operand must be Bool",
                        anchor,
                    ));
                }

                self.require_local_id(*candidate_result, anchor)?;
                self.require_local_id(*in_range_result, anchor)?;
                let Some(candidate_type) = self.local_types.get(candidate_result).copied() else {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate destination has no registered type",
                        anchor,
                    ));
                };
                let Some(in_range_type) = self.local_types.get(in_range_result).copied() else {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate Bool result has no registered type",
                        anchor,
                    ));
                };
                if candidate_type != domain_type
                    || in_range_type != self.type_environment.builtins().bool
                {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate result locals have the wrong types",
                        anchor,
                    ));
                }
            }

            HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            } => {
                self.validate_float_effect_statement(
                    "FormatFloat",
                    source,
                    *failure_mode,
                    *result,
                    self.type_environment.builtins().string,
                    anchor,
                )?;
            }

            HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            } => {
                self.validate_float_effect_statement(
                    "ValidateFloat",
                    source,
                    *failure_mode,
                    *result,
                    self.type_environment.builtins().float,
                    anchor,
                )?;
            }
        }

        Ok(())
    }

    /// WHAT: checks numeric cast inputs against their policy and returns the policy's target type.
    /// WHY: the cast policy owns source/target identity, so validation must reject HIR whose
    ///      expression types drift from that contract before borrow analysis or backend lowering.
    pub(super) fn validate_numeric_cast_source_type(
        &self,
        policy: BuiltinCastPolicyId,
        source: &HirExpression,
        anchor: Option<HirLocation>,
    ) -> Result<Option<TypeId>, CompilerError> {
        let builtins = self.type_environment.builtins();
        let (policy_source_type, policy_target_type) = match policy {
            BuiltinCastPolicyId::NumericConversion { source, target } => (
                self.numeric_scalar_type_id(source, anchor)?,
                self.numeric_scalar_type_id(target, anchor)?,
            ),
            BuiltinCastPolicyId::NumericToString(source) => (
                self.numeric_scalar_type_id(source, anchor)?,
                builtins.string,
            ),
            BuiltinCastPolicyId::StringToNumeric(target) => (
                builtins.string,
                self.numeric_scalar_type_id(target, anchor)?,
            ),
            _ => return Ok(None),
        };

        if source.ty != policy_source_type {
            return Err(self.error_with_hir(
                format!("Cast policy {policy:?} source type does not match the policy source type"),
                anchor,
            ));
        }

        Ok(Some(policy_target_type))
    }

    fn numeric_scalar_type_id(
        &self,
        scalar: NumericScalar,
        anchor: Option<HirLocation>,
    ) -> Result<TypeId, CompilerError> {
        match scalar {
            NumericScalar::Number(scale) => {
                let identity = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Number(scale));
                self.type_environment
                    .type_id_for_canonical_identity(&identity)
                    .ok_or_else(|| {
                        self.error_with_hir(
                            format!("Numeric policy refers to unregistered Dec scale {scale}"),
                            anchor,
                        )
                    })
            }
            _ => Ok(scalar.type_id(self.type_environment)),
        }
    }

    /// Validates Dec-bearing policy rows without expanding scale pairs into a registry.
    ///
    /// WHAT: resolves the exact pair through builtin evidence and checks the required
    ///       infallibility/fallibility for expression casts versus effectful CastOps.
    /// WHY: Dec scale identities are lazy, while HIR must still reject forged policy facts
    ///      before any lowerer consumes them.
    pub(super) fn validate_number_cast_policy(
        &self,
        policy: BuiltinCastPolicyId,
        required_fallibility: BuiltinCastFallibility,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let pair = match policy {
            BuiltinCastPolicyId::NumericConversion { source, target }
                if matches!(source, NumericScalar::Number(_))
                    || matches!(target, NumericScalar::Number(_)) =>
            {
                Some((
                    BuiltinCastTarget::from(source),
                    BuiltinCastTarget::from(target),
                ))
            }
            BuiltinCastPolicyId::NumericToString(NumericScalar::Number(scale)) => {
                Some((BuiltinCastTarget::Number(scale), BuiltinCastTarget::String))
            }
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Number(scale)) => {
                Some((BuiltinCastTarget::String, BuiltinCastTarget::Number(scale)))
            }
            _ => None,
        };
        let Some((source, target)) = pair else {
            return Ok(());
        };

        // Dec-bearing rows are profile-independent: integer/scale direction fixes exact
        // conversion fallibility, and numeric text conversions use fixed policies.
        let Some(row) = lookup_builtin_evidence(source, target, NumericProfile::STANDARD) else {
            return Err(self.error_with_hir(
                format!("Cast policy {policy:?} has no builtin evidence row"),
                anchor,
            ));
        };
        if row.policy != policy {
            return Err(self.error_with_hir(
                format!("Cast policy {policy:?} does not match its builtin evidence row"),
                anchor,
            ));
        }
        if row.fallibility != required_fallibility {
            return Err(self.error_with_hir(
                format!("Cast policy {policy:?} has the wrong fallibility"),
                anchor,
            ));
        }

        Ok(())
    }

    fn validate_number_cast_policy_fallibility_for_statement(
        &self,
        policy: BuiltinCastPolicyId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let required_fallibility = match policy {
            BuiltinCastPolicyId::NumericConversion { .. }
            | BuiltinCastPolicyId::StringToNumeric(_) => BuiltinCastFallibility::Fallible,
            BuiltinCastPolicyId::NumericToString(_) => BuiltinCastFallibility::Infallible,
            _ => return Ok(()),
        };

        self.validate_number_cast_policy(policy, required_fallibility, anchor)
    }

    fn validate_numeric_cast_result_type(
        &self,
        policy: BuiltinCastPolicyId,
        result_type: TypeId,
        expected_success_type: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        match policy {
            BuiltinCastPolicyId::NumericConversion { .. }
            | BuiltinCastPolicyId::StringToNumeric(_) => {
                let Some((carrier_success_type, carrier_error_type)) =
                    self.type_environment.fallible_carrier_slots(result_type)
                else {
                    return Err(self.error_with_hir(
                        format!(
                            "CastOp {policy:?} result local must have an internal fallible carrier type"
                        ),
                        anchor,
                    ));
                };

                if carrier_success_type != expected_success_type {
                    return Err(self.error_with_hir(
                        format!("CastOp {policy:?} carrier has the wrong success type"),
                        anchor,
                    ));
                }

                let builtin_error_identity =
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error);
                let Some(builtin_error_type) = self
                    .type_environment
                    .type_id_for_canonical_identity(&builtin_error_identity)
                else {
                    return Err(self.error_with_hir(
                        format!("CastOp {policy:?} carrier requires the builtin Error type"),
                        anchor,
                    ));
                };

                if carrier_error_type != builtin_error_type {
                    return Err(self.error_with_hir(
                        format!("CastOp {policy:?} carrier has the wrong error type"),
                        anchor,
                    ));
                }
            }

            BuiltinCastPolicyId::NumericToString(_) if result_type != expected_success_type => {
                return Err(self.error_with_hir(
                    format!("CastOp {policy:?} result local has the wrong target type"),
                    anchor,
                ));
            }

            _ => {}
        }

        Ok(())
    }

    fn validate_float_effect_statement(
        &self,
        statement_name: &'static str,
        source: &HirExpression,
        failure_mode: NumericFailureMode,
        result: LocalId,
        success_type: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.validate_expression(source, anchor)?;

        if source.ty != self.type_environment.builtins().float {
            return Err(self.error_with_hir(
                format!("{statement_name} source must have Float type"),
                anchor,
            ));
        }

        self.require_local_id(result, anchor)?;
        let Some(result_type) = self.local_types.get(&result).copied() else {
            return Err(self.error_with_hir(
                format!("{statement_name} result local has no registered type"),
                anchor,
            ));
        };

        match failure_mode {
            NumericFailureMode::Trap => {
                if result_type != success_type {
                    return Err(self.error_with_hir(
                        format!("{statement_name} Trap result local has the wrong success type"),
                        anchor,
                    ));
                }
            }

            NumericFailureMode::ReturnError => {
                let Some((carrier_success_type, _)) =
                    self.type_environment.fallible_carrier_slots(result_type)
                else {
                    return Err(self.error_with_hir(
                        format!(
                            "{statement_name} ReturnError result local must have an internal fallible carrier type"
                        ),
                        anchor,
                    ));
                };

                if carrier_success_type != success_type {
                    return Err(self.error_with_hir(
                        format!("{statement_name} ReturnError carrier has the wrong success type"),
                        anchor,
                    ));
                }
            }
        }

        Ok(())
    }

    pub(super) fn validate_terminator(
        &self,
        block_id: BlockId,
        terminator: &HirTerminator,
    ) -> Result<(), CompilerError> {
        let anchor = Some(HirLocation::Terminator(block_id));

        match terminator {
            HirTerminator::Jump { target, args } => {
                self.require_block_id(*target, anchor)?;
                self.require_same_function_cfg_owner(block_id, *target, anchor)?;
                for local in args {
                    self.require_local_id(*local, anchor)?;
                }
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                self.validate_expression(condition, anchor)?;
                self.require_block_id(*then_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *then_block, anchor)?;
                self.require_block_id(*else_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *else_block, anchor)?;
            }

            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                self.validate_expression(result, anchor)?;
                if self
                    .type_environment
                    .fallible_carrier_slots(result.ty)
                    .is_none()
                {
                    return Err(self.error_with_hir(
                        "FallibleBranch result expression must have an internal fallible carrier type",
                        anchor,
                    ));
                }
                self.require_block_id(*success_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *success_block, anchor)?;
                self.require_block_id(*error_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *error_block, anchor)?;
            }

            HirTerminator::Match { scrutinee, arms } => {
                self.validate_expression(scrutinee, anchor)?;
                let pattern_subject_type_id = self
                    .type_environment
                    .option_inner_type(scrutinee.ty)
                    .unwrap_or(scrutinee.ty);
                for arm in arms {
                    self.validate_match_arm(block_id, arm, pattern_subject_type_id, anchor)?;
                }
            }

            HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                self.require_block_id(*target, anchor)?;
                self.require_same_function_cfg_owner(block_id, *target, anchor)?;
            }

            HirTerminator::Return(value) => {
                self.validate_expression(value, anchor)?;
            }

            HirTerminator::ReturnSuccess(value) => {
                self.validate_expression(value, anchor)?;
                self.validate_fallible_return_terminator(
                    block_id,
                    value,
                    FallibleReturnSlot::Success,
                    anchor,
                )?;
            }

            HirTerminator::ReturnError(value) => {
                self.validate_expression(value, anchor)?;
                self.validate_fallible_return_terminator(
                    block_id,
                    value,
                    FallibleReturnSlot::Error,
                    anchor,
                )?;
            }

            HirTerminator::Uninitialized => {
                return Err(self.error_with_hir(
                    "Placeholder Uninitialized terminators are not allowed in validated HIR",
                    anchor,
                ));
            }

            HirTerminator::RuntimeFailure { .. } => {
                // Compiler-generated runtime failures are valid terminal terminators.
                // They carry backend-facing text only, not HIR expressions.
            }

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                // Assertion failure is a valid terminal terminator.
                // The message is an ordinary typed optional value evaluated on the failure edge.
                self.validate_expression(message, anchor)?;
                let expected_message_type = self.type_environment.builtins().string;
                if self.type_environment.option_inner_type(message.ty)
                    != Some(expected_message_type)
                {
                    return Err(
                        self.error_with_hir("AssertFailure message must have type String?", anchor)
                    );
                }
                let actual_evaluation = classify_assertion_message_evaluation(message);
                if *message_evaluation != actual_evaluation {
                    return Err(self.error_with_hir(
                        format!(
                            "AssertFailure message evaluation fact {:?} does not match lowered shape {:?}",
                            message_evaluation, actual_evaluation
                        ),
                        anchor,
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_fallible_return_terminator(
        &self,
        block_id: BlockId,
        value: &HirExpression,
        slot: FallibleReturnSlot,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let Some(function_id) = self.block_owner_by_id.get(&block_id).copied() else {
            return Err(self.error_with_hir(
                format!(
                    "Block {block_id} has no function owner for {}",
                    slot.terminator_name()
                ),
                anchor,
            ));
        };
        let function = self
            .module
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .ok_or_else(|| {
                self.error_with_hir(
                    format!(
                        "{} owner function {function_id:?} is missing",
                        slot.terminator_name()
                    ),
                    anchor,
                )
            })?;
        let Some((success_type, error_type)) = self
            .type_environment
            .fallible_carrier_slots(function.return_type)
        else {
            return Err(self.error_with_hir(
                format!(
                    "{} in function {function_id:?} whose return type has no {} slot",
                    slot.terminator_name(),
                    slot.slot_name()
                ),
                anchor,
            ));
        };

        let expected_type = slot.select_type(success_type, error_type);
        if value.ty != expected_type {
            return Err(self.error_with_hir(
                format!(
                    "{} value type {:?} does not match function {} slot {:?}",
                    slot.terminator_name(),
                    value.ty,
                    slot.slot_name(),
                    expected_type
                ),
                anchor,
            ));
        }

        Ok(())
    }
}

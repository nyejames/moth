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
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, negation_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{HirNumericOperands, NumericFailureMode};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirTerminator, classify_assertion_message_evaluation,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use rustc_hash::FxHashSet;

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
                self.validate_statement(statement, block.id)?;
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
        self.validate_operation_mappings(statement.id, statement.span.is_some())
    }

    fn validate_operation_mappings(
        &self,
        id: crate::compiler_frontend::hir::ids::HirNodeId,
        authored: bool,
    ) -> Result<(), CompilerError> {
        // Compiler-generated statements intentionally have no authored span and therefore no
        // side-table source mapping. Authored statements must carry both reversible provenance
        // and the exact diagnostic span.
        if !authored {
            return Ok(());
        }

        let statement_location = HirLocation::Statement(id);
        if self
            .module
            .side_table
            .ast_span_for_hir(statement_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!("Statement {} is missing AST->HIR side-table mapping", id),
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
                format!("Statement {} is missing HIR source side-table mapping", id),
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

    fn statement_function_id(
        &self,
        block_id: BlockId,
        anchor: Option<HirLocation>,
    ) -> Result<crate::compiler_frontend::hir::ids::FunctionId, CompilerError> {
        self.block_owner_by_id
            .get(&block_id)
            .copied()
            .ok_or_else(|| {
                self.error_with_hir(
                    format!("Statement block {block_id} has no function owner"),
                    anchor,
                )
            })
    }

    fn require_statement_local(
        &self,
        local_id: LocalId,
        block_id: BlockId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let function_id = self.statement_function_id(block_id, anchor)?;
        self.require_local_in_function(local_id, function_id, anchor)
    }

    fn require_statement_destination(
        &self,
        destination: HirLocalDestination,
        block_id: BlockId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.require_statement_local(destination.local(), block_id, anchor)?;
        if let HirLocalDestination::Define(local_id) = destination {
            self.require_local_not_parameter_definition(local_id, anchor)?;
        }
        Ok(())
    }

    fn validate_statement_local_destinations(
        &self,
        kind: &HirStatementKind,
        block_id: BlockId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        match kind {
            HirStatementKind::Write { target, .. } => match target {
                HirWriteTarget::DefineLocal(local) => {
                    self.require_statement_local(*local, block_id, anchor)?;
                    self.require_local_not_parameter_definition(*local, anchor)?;
                }
                HirWriteTarget::AssignPlace(place) => {
                    self.require_statement_local(place.root, block_id, anchor)?;
                }
            },
            HirStatementKind::Call { results, .. } => {
                for destination in results {
                    self.require_statement_destination(*destination, block_id, anchor)?;
                }
            }
            HirStatementKind::MapOp {
                result: Some(destination),
                ..
            } => self.require_statement_destination(*destination, block_id, anchor)?,
            HirStatementKind::NumericOp { result, .. }
            | HirStatementKind::RangeStepFailure { result, .. }
            | HirStatementKind::FormatFloat { result, .. }
            | HirStatementKind::ValidateFloat { result, .. } => {
                self.require_statement_destination(*result, block_id, anchor)?;
            }
            HirStatementKind::FloatRangeCandidate {
                candidate_result,
                in_range_result,
                ..
            } => {
                self.require_statement_destination(*candidate_result, block_id, anchor)?;
                self.require_statement_destination(*in_range_result, block_id, anchor)?;
            }
            HirStatementKind::PushRuntimeFragment { vec_local, .. } => {
                self.require_statement_local(*vec_local, block_id, anchor)?;
            }
            HirStatementKind::MapOp { result: None, .. } | HirStatementKind::Expr(_) => {}
        }

        Ok(())
    }

    pub(super) fn validate_statement(
        &self,
        statement: &HirStatement,
        block_id: BlockId,
    ) -> Result<(), CompilerError> {
        let anchor = Some(HirLocation::Statement(statement.id));
        self.validate_statement_local_destinations(&statement.kind, block_id, anchor)?;
        match &statement.kind {
            HirStatementKind::Write { target, value } => {
                let target_type = match target {
                    HirWriteTarget::DefineLocal(local) => {
                        self.require_statement_local(*local, block_id, anchor)?;
                        self.local_types.get(local).copied().ok_or_else(|| {
                            self.error_with_hir(
                                "Write definition local has no registered type",
                                anchor,
                            )
                        })?
                    }
                    HirWriteTarget::AssignPlace(place) => self.validate_place(place, anchor)?,
                };
                self.validate_expression(*value, anchor)?;
                if self.expression_row(*value, anchor)?.ty != target_type {
                    return Err(self.error_with_hir(
                        "Write value type does not match its destination type",
                        anchor,
                    ));
                }
            }

            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                self.validate_call_channels(
                    target,
                    *args,
                    results.iter().map(|result| result.local()),
                    false,
                    anchor,
                )?;
            }

            HirStatementKind::Expr(expression) => {
                self.validate_expression(*expression, anchor)?;
            }

            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => {
                self.validate_map_channels(
                    *op,
                    *receiver,
                    *args,
                    result.iter().map(|result| result.local()),
                    false,
                    anchor,
                )?;
            }

            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                self.require_statement_local(*vec_local, block_id, anchor)?;
                self.validate_expression(*value, anchor)?;
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
                // its exponent in the exact builtin `I32` domain.
                let exponent_domain = NumericScalar::Fixed(FixedScalar::I32);
                let domain_type = self.numeric_scalar_type_id(op.domain, anchor)?;
                let number_power = op.operator == NumericOperator::Power
                    && matches!(op.domain, NumericScalar::Number(_));

                match operands {
                    HirNumericOperands::Unary { operand } => {
                        self.validate_expression(*operand, anchor)?;
                        let operand = self.expression_row(*operand, anchor)?;

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
                        self.validate_expression(*left, anchor)?;
                        self.validate_expression(*right, anchor)?;
                        let left = self.expression_row(*left, anchor)?;
                        let right = self.expression_row(*right, anchor)?;

                        let right_type = if number_power {
                            exponent_domain.type_id(self.type_environment)
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
                        binary_operation_domain(op.operator, op.domain, exponent_domain)
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

                self.validate_numeric_result_local(
                    format_args!("NumericOp::{op}"),
                    *failure_mode,
                    *result,
                    domain_type,
                    anchor,
                )?;
            }

            HirStatementKind::RangeStepFailure {
                failure_mode,
                result,
                ..
            } => {
                if *failure_mode == NumericFailureMode::Infallible {
                    return Err(self.error_with_hir(
                        "RangeStepFailure cannot use the discharged numeric mode".to_owned(),
                        anchor,
                    ));
                }
                self.validate_numeric_result_local(
                    "RangeStepFailure",
                    *failure_mode,
                    *result,
                    self.type_environment.builtins().bool,
                    anchor,
                )?;
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
                if !domain.is_binary_float() {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate requires a binary-float domain",
                        anchor,
                    ));
                }

                self.validate_expression(*current, anchor)?;
                self.validate_expression(*step, anchor)?;
                self.validate_expression(*end, anchor)?;
                self.validate_expression(*ascending, anchor)?;

                let domain_type = domain.type_id(self.type_environment);
                let current = self.expression_row(*current, anchor)?;
                let step = self.expression_row(*step, anchor)?;
                let end = self.expression_row(*end, anchor)?;
                let ascending = self.expression_row(*ascending, anchor)?;
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

                let candidate_local = candidate_result.local();
                let in_range_local = in_range_result.local();
                self.require_local_id(candidate_local, anchor)?;
                self.require_local_id(in_range_local, anchor)?;
                let Some(candidate_type) = self.local_types.get(&candidate_local).copied() else {
                    return Err(self.error_with_hir(
                        "FloatRangeCandidate destination has no registered type",
                        anchor,
                    ));
                };
                let Some(in_range_type) = self.local_types.get(&in_range_local).copied() else {
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
                if *failure_mode == NumericFailureMode::Infallible {
                    return Err(self.error_with_hir(
                        "FormatFloat cannot use the discharged numeric mode".to_owned(),
                        anchor,
                    ));
                }
                self.validate_float_effect_statement(
                    "FormatFloat",
                    *source,
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
                if *failure_mode == NumericFailureMode::Infallible {
                    return Err(self.error_with_hir(
                        "ValidateFloat cannot use the discharged numeric mode".to_owned(),
                        anchor,
                    ));
                }
                self.validate_validated_float_statement(*source, *failure_mode, *result, anchor)?;
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
        source: HirValueId,
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

        if self.expression_row(source, anchor)?.ty != policy_source_type {
            return Err(self.error_with_hir(
                format!("Cast policy {policy:?} source type does not match the policy source type"),
                anchor,
            ));
        }

        Ok(Some(policy_target_type))
    }

    pub(super) fn numeric_scalar_type_id(
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

        // Dec-bearing rows are decided by exact scale: integer/scale direction fixes exact
        // conversion fallibility, and numeric text conversions use fixed policies.
        let Some(row) = lookup_builtin_evidence(source, target) else {
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

    fn validate_float_effect_statement(
        &self,
        statement_name: &'static str,
        source: HirValueId,
        failure_mode: NumericFailureMode,
        result: HirLocalDestination,
        success_type: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.validate_expression(source, anchor)?;
        let source = self.expression_row(source, anchor)?;

        if !NumericScalar::from_type_id(source.ty, self.type_environment)
            .is_some_and(NumericScalar::is_binary_float)
        {
            return Err(self.error_with_hir(
                format!("{statement_name} source must have an exact binary-float type"),
                anchor,
            ));
        }

        self.validate_numeric_result_local(
            statement_name,
            failure_mode,
            result,
            success_type,
            anchor,
        )
    }

    /// Validates a `ValidateFloat` statement against its source row's exact binary-float type.
    ///
    /// WHAT: admits every exact `F16`/`F32`/`F64` source, then requires the
    ///       result local to carry exactly that source type.
    /// WHY: the source expression row owns the resolved precision, so the validated result must
    ///      keep it instead of assuming a default float precision.
    fn validate_validated_float_statement(
        &self,
        source: HirValueId,
        failure_mode: NumericFailureMode,
        result: HirLocalDestination,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.validate_expression(source, anchor)?;
        let source_type = self.expression_row(source, anchor)?.ty;

        if !NumericScalar::from_type_id(source_type, self.type_environment)
            .is_some_and(NumericScalar::is_binary_float)
        {
            return Err(self.error_with_hir(
                "ValidateFloat source must have an exact binary-float type",
                anchor,
            ));
        }

        self.validate_numeric_result_local(
            "ValidateFloat",
            failure_mode,
            result,
            source_type,
            anchor,
        )
    }

    fn validate_numeric_result_local(
        &self,
        statement_name: impl std::fmt::Display,
        failure_mode: NumericFailureMode,
        result: HirLocalDestination,
        success_type: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let result_local = result.local();
        self.require_local_id(result_local, anchor)?;
        let Some(result_type) = self.local_types.get(&result_local).copied() else {
            return Err(self.error_with_hir(
                format!("{statement_name} result local has no registered type"),
                anchor,
            ));
        };

        if result_type != success_type {
            return Err(self.error_with_hir(
                format!(
                    "{statement_name} {failure_mode:?} result local has the wrong success type"
                ),
                anchor,
            ));
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
                let function_id =
                    self.block_owner_by_id
                        .get(&block_id)
                        .copied()
                        .ok_or_else(|| {
                            self.error_with_hir(
                                format!("Jump source block {block_id} has no function owner"),
                                anchor,
                            )
                        })?;
                let mut destinations = FxHashSet::default();
                for argument in args {
                    self.require_local_in_function(argument.source, function_id, anchor)?;
                    self.require_local_in_function(argument.destination, function_id, anchor)?;
                    self.require_local_in_block(argument.destination, *target, anchor)?;
                    self.require_local_not_parameter_definition(argument.destination, anchor)?;
                    if !destinations.insert(argument.destination) {
                        return Err(self.error_with_hir(
                            format!(
                                "Jump to block {target} defines destination {:?} more than once",
                                argument.destination
                            ),
                            anchor,
                        ));
                    }
                    let source_type =
                        self.local_types
                            .get(&argument.source)
                            .copied()
                            .ok_or_else(|| {
                                self.error_with_hir(
                                    format!(
                                        "Jump source local {:?} has no registered type",
                                        argument.source
                                    ),
                                    anchor,
                                )
                            })?;
                    let destination_type = self
                        .local_types
                        .get(&argument.destination)
                        .copied()
                        .ok_or_else(|| {
                            self.error_with_hir(
                                format!(
                                    "Jump destination local {:?} has no registered type",
                                    argument.destination
                                ),
                                anchor,
                            )
                        })?;
                    if source_type != destination_type {
                        return Err(self.error_with_hir(
                            format!(
                                "Jump argument source {:?} and destination {:?} have different types",
                                argument.source, argument.destination
                            ),
                            anchor,
                        ));
                    }
                }
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                self.validate_expression(*condition, anchor)?;
                self.require_block_id(*then_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *then_block, anchor)?;
                self.require_block_id(*else_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *else_block, anchor)?;
            }

            HirTerminator::Invoke { id, span, .. } => {
                self.validate_operation_mappings(*id, span.is_some())?;
                self.validate_invoke(block_id, terminator, anchor)?;
            }

            HirTerminator::Match { scrutinee, arms } => {
                self.validate_expression(*scrutinee, anchor)?;
                let scrutinee = self.expression_row(*scrutinee, anchor)?;
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

            HirTerminator::Return(values) => {
                let values = self.value_ids(*values, anchor)?;
                let function_id = self.statement_function_id(block_id, anchor)?;
                let function = self.module.functions.iter().find(|function| function.id == function_id)
                    .ok_or_else(|| self.error_with_hir("Return owner function is missing", anchor))?;
                if values.len() != function.returns.len() {
                    return Err(self.error_with_hir("Return success slot arity does not match function signature", anchor));
                }
                for (value, expected) in values.iter().zip(function.returns.iter()) {
                    self.validate_expression(*value, anchor)?;
                    if self.expression_row(*value, anchor)?.ty != *expected {
                        return Err(self.error_with_hir("Return success slot type does not match function signature", anchor));
                    }
                }
            }

            HirTerminator::ReturnError(value) => {
                self.validate_expression(*value, anchor)?;
                let function_id = self.statement_function_id(block_id, anchor)?;
                let function = self.module.functions.iter().find(|function| function.id == function_id)
                    .ok_or_else(|| self.error_with_hir("ReturnError owner function is missing", anchor))?;
                if function.error_return != Some(self.expression_row(*value, anchor)?.ty) {
                    return Err(self.error_with_hir("ReturnError value type does not match function error slot", anchor));
                }
            }

            HirTerminator::Uninitialized => {
                return Err(self.error_with_hir(
                    "Placeholder Uninitialized terminators are not allowed in validated HIR",
                    anchor,
                ));
            }

            HirTerminator::RuntimeFailure { cause, .. } => {
                if let Some(
                    crate::compiler_frontend::hir::terminators::RuntimeFailureCause::StoreConversion { error }
                    | crate::compiler_frontend::hir::terminators::RuntimeFailureCause::AuthoredCastConversion { error }
                ) = cause {
                    self.require_statement_local(*error, block_id, anchor)?;
                    if self.local_types.get(error) != Some(&self.builtin_error_type(anchor)?) {
                        return Err(self.error_with_hir("RuntimeFailure conversion cause must reference builtin Error", anchor));
                    }
                }
            }

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                // Assertion failure is a valid terminal terminator.
                // The message is an ordinary typed optional value evaluated on the failure edge.
                let message_id = *message;
                self.validate_expression(message_id, anchor)?;
                let message = self.expression_row(message_id, anchor)?;
                let expected_message_type = self.type_environment.builtins().string;
                if self.type_environment.option_inner_type(message.ty)
                    != Some(expected_message_type)
                {
                    return Err(
                        self.error_with_hir("AssertFailure message must have type String?", anchor)
                    );
                }
                let actual_evaluation =
                    classify_assertion_message_evaluation(&self.module.expressions, message_id);
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
}

//! JS emitter — lowers HIR into executable JavaScript.
//!
//! WHAT: converts HIR control flow/expressions into executable JS and symbol maps.
//! WHY: JS is the stable near-term backend and needs deterministic lowering output.

use crate::backends::js::JsModule;
use crate::backends::js::numeric_carrier::JsNumericCarrier;
use crate::backends::js::runtime::NumericRuntimeHelperUsage;
use crate::backends::js::{ENTRY_FAILURE_NOTICE, JsFunctionEmissionPolicy, JsLoweringConfig};
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FieldId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::{HashMap, HashSet};

/// Lower one validated HIR module into JavaScript source.
pub fn lower_hir_to_js(
    hir: &HirModule,
    numeric_proofs: &NumericProofs,
    string_table: &StringTable,
    config: JsLoweringConfig,
    type_environment: &TypeEnvironment,
    path_table: &PathTable,
) -> Result<JsModule, CompilerError> {
    let emitter = JsEmitter::new(
        hir,
        numeric_proofs,
        string_table,
        path_table,
        config,
        type_environment,
    );
    emitter.lower_module()
}

pub(crate) struct JsEmitter<'hir> {
    pub(crate) hir: &'hir HirModule,
    /// Proven integer operation/narrowing facts paired with this immutable HIR. An empty table
    /// retains every checked numeric statement unchanged.
    pub(crate) numeric_proofs: &'hir NumericProofs,
    pub(crate) string_table: &'hir StringTable,
    pub(crate) path_table: &'hir PathTable,
    pub(crate) config: JsLoweringConfig,
    pub(crate) type_environment: &'hir TypeEnvironment,
    pub(crate) out: String,
    pub(crate) indent: usize,
    pub(crate) blocks_by_id: HashMap<BlockId, &'hir HirBlock>,
    pub(crate) function_name_by_id: HashMap<FunctionId, String>,
    pub(crate) local_name_by_id: HashMap<LocalId, String>,
    pub(crate) field_name_by_id: HashMap<FieldId, String>,
    pub(crate) current_function: Option<FunctionId>,
    pub(crate) used_identifiers: HashSet<String>,
    pub(crate) temp_counter: usize,
    /// Set of external function IDs referenced while lowering emitted JS functions.
    /// Used to conditionally emit runtime helpers.
    pub(crate) referenced_external_functions:
        HashSet<crate::compiler_frontend::external_packages::ExternalFunctionId>,
    /// Whether choice equality was lowered, requiring the runtime helper.
    pub(crate) used_choice_equality: bool,
    /// Whether an emitted assertion terminator needs the structural fault helper.
    pub(crate) used_assertions: bool,
    /// Builtin cast policies collected pre-emission by `collect_used_cast_policies`.
    /// Used to conditionally emit the matching runtime helpers.
    pub(crate) used_cast_policies: HashSet<BuiltinCastPolicyId>,
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn new(
        hir: &'hir HirModule,
        numeric_proofs: &'hir NumericProofs,
        string_table: &'hir StringTable,
        path_table: &'hir PathTable,
        config: JsLoweringConfig,
        type_environment: &'hir TypeEnvironment,
    ) -> Self {
        let blocks_by_id = hir
            .blocks
            .iter()
            .map(|block| (block.id, block))
            .collect::<HashMap<_, _>>();

        Self {
            out: String::new(),
            hir,
            numeric_proofs,
            string_table,
            path_table,
            config,
            type_environment,
            indent: 0,
            blocks_by_id,
            function_name_by_id: HashMap::new(),
            local_name_by_id: HashMap::new(),
            field_name_by_id: HashMap::new(),
            current_function: None,
            used_identifiers: HashSet::new(),
            temp_counter: 0,
            referenced_external_functions: HashSet::new(),
            used_choice_equality: false,
            used_assertions: false,
            used_cast_policies: HashSet::new(),
        }
    }

    fn lower_module(mut self) -> Result<JsModule, CompilerError> {
        if let JsFunctionEmissionPolicy::Selected(selection) = &self.config.function_emission_policy
        {
            selection.validate_for_hir(self.hir)?;
        }
        self.build_symbol_maps()?;

        let functions = self.functions_to_emit();
        let start_is_fallible = functions.iter().any(|function| {
            Some(function.id) == self.hir.start_function && self.function_is_fallible(function)
        });
        let emitted_code_uses_maps = self.emitted_functions_use_maps(&functions)?;
        let mut emitted_code_uses_numeric_helpers =
            self.emitted_functions_use_numeric_helpers(&functions)?;
        self.collect_used_cast_policies(&functions)?;
        for policy in &self.used_cast_policies {
            if let BuiltinCastPolicyId::NumericToString(scalar) = *policy
                && let Some(precision) = scalar.binary_float_precision(self.config.numeric_profile)
            {
                emitted_code_uses_numeric_helpers.require_float_formatter(precision);
            }
        }
        self.emit_runtime_prelude(emitted_code_uses_maps, emitted_code_uses_numeric_helpers);

        for (index, function) in functions.into_iter().enumerate() {
            if index > 0 {
                self.emit_line("");
            }

            self.emit_function(function)?;
        }

        self.emit_core_package_helpers();

        if self.used_choice_equality {
            self.emit_runtime_choice_helpers();
        }

        if self.used_assertions {
            self.emit_runtime_assertion_helper();
        }

        if self.config.auto_invoke_start {
            let start_function = self
                .hir
                .require_start_function("JavaScript automatic start invocation")?;
            let Some(start_name) = self.function_name_by_id.get(&start_function).cloned() else {
                return Err(CompilerError::compiler_error(format!(
                    "JavaScript backend: start function {:?} has no generated JS name",
                    start_function
                )));
            };

            if !self.out.is_empty() {
                self.emit_line("");
            }

            if start_is_fallible {
                // Keep typed entry failure local to this invocation, without host exceptions.
                self.emit_line("(function () {");
                self.indent += 1;
                self.emit_line(&format!("var moth_result = {start_name}();"));
                self.emit_line("if (moth_result.tag !== \"ok\") {");
                self.indent += 1;
                self.emit_line(
                    "if (typeof globalThis.__moth_record_entry_failure === \"function\") {",
                );
                self.indent += 1;
                self.emit_line("var moth_error = moth_result.value;");
                self.emit_line(
                    "globalThis.__moth_record_entry_failure(__moth_error_code(moth_error), __moth_error_message(moth_error), moth_error.__moth_location ?? null);",
                );
                self.indent -= 1;
                self.emit_line("}");
                self.emit_line("if (typeof process !== \"undefined\" && process.stderr) {");
                self.indent += 1;
                self.emit_line(&format!("process.stderr.write({ENTRY_FAILURE_NOTICE:?});"));
                self.emit_line("process.exitCode = 1;");
                self.indent -= 1;
                self.emit_line("}");
                self.emit_line("return;");
                self.indent -= 1;
                self.emit_line("}");
                self.indent -= 1;
                self.emit_line("})();");
            } else {
                self.emit_line(&format!("{start_name}();"));
            }
        }

        Ok(JsModule {
            source: self.out,
            function_name_by_id: self.function_name_by_id,
            start_is_fallible,
            referenced_external_functions: self.referenced_external_functions,
        })
    }

    fn functions_to_emit(&self) -> Vec<&'hir HirFunction> {
        let mut functions = self
            .hir
            .functions
            .iter()
            .filter(|function| self.config.function_emission_policy.includes(function.id))
            .collect::<Vec<_>>();
        functions.sort_by_key(|function| function.id.0);

        functions
    }

    /// Records the carrier families needed by reachable numeric statements.
    ///
    /// WHAT: scans the emitted function/block subset and selects each checked operation family
    ///       through the same profile-aware carrier owner used by expression and cast lowering.
    /// WHY: a BigInt operator must not inherit a Number helper merely because both domains are
    ///      semantically integers.
    fn emitted_functions_use_numeric_helpers(
        &self,
        functions: &[&'hir HirFunction],
    ) -> Result<NumericRuntimeHelperUsage, CompilerError> {
        let mut usage = NumericRuntimeHelperUsage::default();

        for function in functions {
            let reachable_blocks = self.collect_reachable_blocks(function.entry)?;
            for block_id in reachable_blocks {
                let block = self.block_by_id(block_id)?;
                for statement in &block.statements {
                    match &statement.kind {
                        HirStatementKind::NumericOp { op, .. } => {
                            let carrier = JsNumericCarrier::for_scalar(
                                op.domain,
                                self.config.numeric_profile,
                            )
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "JS backend has no numeric carrier for {:?}",
                                    op.domain
                                ))
                            })?;
                            let helper_family = carrier.helper_family();
                            // WHAT: proven-safe integer operations lower to exact native carrier
                            //       arithmetic without any checked helper call — Trap mode as a
                            //       raw scalar, ReturnError mode inside the existing
                            //       `{tag, value}` success carrier its HIR consumers branch on.
                            // WHY: their helper family is needed only by retained operations, so
                            //       families used exclusively by proven statements stay out of
                            //       the prelude while genuinely unsafe sibling uses keep their
                            //       demand.
                            let proven_safe = matches!(
                                op.operator,
                                NumericOperator::Add
                                    | NumericOperator::Subtract
                                    | NumericOperator::Multiply
                                    | NumericOperator::IntegerDivide
                                    | NumericOperator::Remainder
                                    | NumericOperator::Negate
                            ) && matches!(
                                carrier,
                                JsNumericCarrier::ExactInteger { .. }
                                    | JsNumericCarrier::BigInteger { .. }
                            ) && self.numeric_proofs.integer_operation_is_safe(
                                statement.id,
                                self.config.numeric_profile,
                            );
                            if !proven_safe {
                                match helper_family {
                                    Some("int") => usage.number_integer_ops = true,
                                    Some("number") => usage.number_decimal_ops = true,
                                    Some("bigint") => usage.big_integer_ops = true,
                                    Some("float32") => usage.binary32_ops = true,
                                    Some("float") => usage.binary64_ops = true,
                                    _ => {
                                        return Err(CompilerError::compiler_error(format!(
                                            "JS backend received an unreachable numeric operation domain {:?}",
                                            op.domain
                                        )));
                                    }
                                }
                            }
                            if op.operator == NumericOperator::Power
                                && matches!(helper_family, Some("float32") | Some("float"))
                            {
                                usage.binary_float_power = true;
                            }
                        }
                        HirStatementKind::FormatFloat { .. } => usage.require_float_formatter(
                            self.config.numeric_profile.float_precision.into(),
                        ),
                        HirStatementKind::ValidateFloat { .. } => usage.validate_float = true,
                        HirStatementKind::RangeStepFailure { .. } => {
                            usage.range_step_failure = true
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(usage)
    }

    /// Returns true when any emitted reachable JS body can construct, store, copy, display, or
    /// operate on a Moth map.
    ///
    /// WHAT: scans the same function/block subset that JS lowering will emit.
    /// WHY: map helpers are new runtime surface. Emitting them only for map-using programs avoids
    /// changing existing golden artifacts while still making map copy/string fallback paths safe.
    fn emitted_functions_use_maps(
        &self,
        functions: &[&'hir HirFunction],
    ) -> Result<bool, CompilerError> {
        for function in functions {
            if self.type_environment.is_map_type(function.return_type) {
                return Ok(true);
            }

            let reachable_blocks = self.collect_reachable_blocks(function.entry)?;
            for block_id in reachable_blocks {
                let block = self.block_by_id(block_id)?;
                for local in &block.locals {
                    if self.type_environment.is_map_type(local.ty) {
                        return Ok(true);
                    }
                }

                for statement in &block.statements {
                    if self.statement_uses_maps(&statement.kind) {
                        return Ok(true);
                    }
                }

                if self.terminator_uses_maps(&block.terminator) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn statement_uses_maps(&self, statement: &HirStatementKind) -> bool {
        match statement {
            HirStatementKind::Write { target, value } => {
                let place_uses_maps = match target {
                    HirWriteTarget::DefineLocal(_) => false,
                    HirWriteTarget::AssignPlace(place) => self.place_uses_maps(*place),
                };
                place_uses_maps || self.expression_uses_maps(*value)
            }

            HirStatementKind::Expr(value) | HirStatementKind::PushRuntimeFragment { value, .. } => {
                self.expression_uses_maps(*value)
            }

            HirStatementKind::Call { args, .. } => self
                .hir
                .expressions
                .values(*args)
                .iter()
                .any(|argument| self.expression_uses_maps(*argument)),

            HirStatementKind::CastOp { source, .. } => self.expression_uses_maps(*source),

            HirStatementKind::FormatFloat { source, .. }
            | HirStatementKind::ValidateFloat { source, .. } => self.expression_uses_maps(*source),

            HirStatementKind::MapOp { .. } => true,

            HirStatementKind::NumericOp { operands, .. } => {
                self.numeric_operands_use_maps(operands)
            }

            HirStatementKind::FloatRangeCandidate {
                current,
                step,
                end,
                ascending,
                ..
            } => {
                self.expression_uses_maps(*current)
                    || self.expression_uses_maps(*step)
                    || self.expression_uses_maps(*end)
                    || self.expression_uses_maps(*ascending)
            }
            HirStatementKind::RangeStepFailure { .. } | HirStatementKind::Drop(_) => false,
        }
    }

    fn numeric_operands_use_maps(&self, operands: &HirNumericOperands) -> bool {
        match operands {
            HirNumericOperands::Unary { operand } => self.expression_uses_maps(*operand),
            HirNumericOperands::Binary { left, right } => {
                self.expression_uses_maps(*left) || self.expression_uses_maps(*right)
            }
        }
    }

    /// Records builtin cast policies used by the functions that will be emitted.
    ///
    /// WHAT: scans the same reachable function/body subset selected for JS output before the
    /// runtime prelude is written.
    /// WHY: cast runtime helpers are emitted in the prelude, so policy discovery cannot depend on
    /// the later expression-lowering pass that writes function bodies.
    fn collect_used_cast_policies(
        &mut self,
        functions: &[&'hir HirFunction],
    ) -> Result<(), CompilerError> {
        self.used_cast_policies.clear();

        for function in functions {
            let reachable_blocks = self.collect_reachable_blocks(function.entry)?;
            for block_id in reachable_blocks {
                let block = self.block_by_id(block_id)?;

                for statement in &block.statements {
                    match &statement.kind {
                        // WHAT: a proven-safe fallible integer narrowing lowers to a direct
                        //       carrier conversion without helper calls, so only the source
                        //       expression's own cast demands remain.
                        // WHY: any retained use of the same policy elsewhere still inserts it
                        //       through the ordinary walk, so its helpers stay demanded.
                        HirStatementKind::CastOp { policy, source, .. }
                            if self.integer_narrowing_is_proven(statement.id, *policy) =>
                        {
                            collect_expression_cast_policies(
                                self.hir,
                                *source,
                                &mut self.used_cast_policies,
                            );
                        }
                        kind => {
                            collect_statement_cast_policies(
                                self.hir,
                                kind,
                                &mut self.used_cast_policies,
                            );
                        }
                    }
                }

                collect_terminator_cast_policies(
                    self.hir,
                    &block.terminator,
                    &mut self.used_cast_policies,
                );
            }
        }

        Ok(())
    }

    fn terminator_uses_maps(&self, terminator: &HirTerminator) -> bool {
        match terminator {
            HirTerminator::If { condition, .. } => self.expression_uses_maps(*condition),

            HirTerminator::FallibleBranch { result, .. } => self.expression_uses_maps(*result),

            HirTerminator::Match { scrutinee, arms } => {
                self.expression_uses_maps(*scrutinee)
                    || arms.iter().any(|arm| {
                        arm.guard
                            .is_some_and(|guard| self.expression_uses_maps(guard))
                            || self.pattern_uses_maps(&arm.pattern)
                    })
            }

            HirTerminator::Return(value)
            | HirTerminator::ReturnSuccess(value)
            | HirTerminator::ReturnError(value) => self.expression_uses_maps(*value),

            HirTerminator::AssertFailure { message, .. } => self.expression_uses_maps(*message),

            HirTerminator::Jump { .. }
            | HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::Uninitialized
            | HirTerminator::RuntimeFailure { .. } => false,
        }
    }

    fn pattern_uses_maps(&self, pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Literal(value)
            | HirPattern::OptionValue { value }
            | HirPattern::OptionRelational { value, .. }
            | HirPattern::Relational { value, .. } => self.expression_uses_maps(*value),

            HirPattern::OptionNone
            | HirPattern::OptionPresent
            | HirPattern::Wildcard
            | HirPattern::ChoiceVariant { .. } => false,
        }
    }

    fn expression_uses_maps(&self, expression_id: HirValueId) -> bool {
        let hir = self.hir;
        let expression = hir.expressions.expression(expression_id);
        if self.type_environment.is_map_type(expression.ty) {
            return true;
        }

        match &expression.kind {
            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
                self.place_uses_maps(*place)
            }

            HirExpressionKind::BinOp { left, right, .. } => {
                self.expression_uses_maps(*left) || self.expression_uses_maps(*right)
            }

            HirExpressionKind::UnaryOp { operand, .. } => self.expression_uses_maps(*operand),

            HirExpressionKind::StructConstruct { fields, .. } => hir
                .expressions
                .struct_fields(*fields)
                .iter()
                .any(|(_, field_value)| self.expression_uses_maps(*field_value)),

            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => hir
                .expressions
                .values(*elements)
                .iter()
                .any(|element| self.expression_uses_maps(*element)),

            HirExpressionKind::MapLiteral(_) => true,

            HirExpressionKind::Range { start, end } => {
                self.expression_uses_maps(*start) || self.expression_uses_maps(*end)
            }

            HirExpressionKind::TupleGet { tuple, .. } => self.expression_uses_maps(*tuple),

            HirExpressionKind::FallibleUnwrapSuccess { result }
            | HirExpressionKind::FallibleUnwrapError { result } => {
                self.expression_uses_maps(*result)
            }

            HirExpressionKind::Cast { source, .. } => self.expression_uses_maps(*source),

            HirExpressionKind::VariantConstruct { fields, .. } => hir
                .expressions
                .variant_fields(*fields)
                .iter()
                .any(|field| self.expression_uses_maps(field.value)),

            HirExpressionKind::VariantPayloadGet { source, .. } => {
                self.expression_uses_maps(*source)
            }

            HirExpressionKind::Int(_)
            | HirExpressionKind::Uint(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::FixedScalar(_)
            | HirExpressionKind::Number(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_)
            | HirExpressionKind::StructuralString { .. } => false,
        }
    }

    fn place_uses_maps(&self, place: HirPlace) -> bool {
        self.hir
            .expressions
            .projections(place.projections)
            .iter()
            .any(|projection| match projection {
                HirProjection::Field(_) => false,
                HirProjection::Index(index) => self.expression_uses_maps(*index),
            })
    }
}

fn collect_statement_cast_policies(
    hir: &HirModule,
    statement: &HirStatementKind,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match statement {
        HirStatementKind::Write { target, value } => {
            if let HirWriteTarget::AssignPlace(place) = target {
                collect_place_cast_policies(hir, *place, policies);
            }
            collect_expression_cast_policies(hir, *value, policies);
        }
        HirStatementKind::Expr(value) | HirStatementKind::PushRuntimeFragment { value, .. } => {
            collect_expression_cast_policies(hir, *value, policies);
        }
        HirStatementKind::Call { args, .. } => {
            for argument in hir.expressions.values(*args) {
                collect_expression_cast_policies(hir, *argument, policies);
            }
        }
        HirStatementKind::CastOp { policy, source, .. } => {
            policies.insert(*policy);
            collect_expression_cast_policies(hir, *source, policies);
        }
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            collect_expression_cast_policies(hir, *source, policies);
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            collect_expression_cast_policies(hir, *receiver, policies);
            for argument in hir.expressions.values(*args) {
                collect_expression_cast_policies(hir, *argument, policies);
            }
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => {
                collect_expression_cast_policies(hir, *operand, policies);
            }
            HirNumericOperands::Binary { left, right } => {
                collect_expression_cast_policies(hir, *left, policies);
                collect_expression_cast_policies(hir, *right, policies);
            }
        },
        HirStatementKind::FloatRangeCandidate {
            current,
            step,
            end,
            ascending,
            ..
        } => {
            collect_expression_cast_policies(hir, *current, policies);
            collect_expression_cast_policies(hir, *step, policies);
            collect_expression_cast_policies(hir, *end, policies);
            collect_expression_cast_policies(hir, *ascending, policies);
        }
        HirStatementKind::RangeStepFailure { .. } | HirStatementKind::Drop(_) => {}
    }
}

fn collect_terminator_cast_policies(
    hir: &HirModule,
    terminator: &HirTerminator,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match terminator {
        HirTerminator::If { condition, .. } => {
            collect_expression_cast_policies(hir, *condition, policies)
        }
        HirTerminator::FallibleBranch { result, .. } => {
            collect_expression_cast_policies(hir, *result, policies);
        }
        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_cast_policies(hir, *scrutinee, policies);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    collect_expression_cast_policies(hir, guard, policies);
                }
                collect_pattern_cast_policies(hir, &arm.pattern, policies);
            }
        }
        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => {
            collect_expression_cast_policies(hir, *value, policies)
        }
        HirTerminator::AssertFailure { message, .. } => {
            collect_expression_cast_policies(hir, *message, policies)
        }
        HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::RuntimeFailure { .. } => {}
    }
}

fn collect_pattern_cast_policies(
    hir: &HirModule,
    pattern: &HirPattern,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match pattern {
        HirPattern::Literal(value)
        | HirPattern::OptionValue { value }
        | HirPattern::OptionRelational { value, .. }
        | HirPattern::Relational { value, .. } => {
            collect_expression_cast_policies(hir, *value, policies)
        }
        HirPattern::OptionNone
        | HirPattern::OptionPresent
        | HirPattern::Wildcard
        | HirPattern::ChoiceVariant { .. } => {}
    }
}

fn collect_expression_cast_policies(
    hir: &HirModule,
    expression_id: HirValueId,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    let expression = hir.expressions.expression(expression_id);
    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            collect_expression_cast_policies(hir, *left, policies);
            collect_expression_cast_policies(hir, *right, policies);
        }
        HirExpressionKind::UnaryOp { operand, .. } => {
            collect_expression_cast_policies(hir, *operand, policies)
        }
        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in hir.expressions.struct_fields(*fields) {
                collect_expression_cast_policies(hir, *value, policies);
            }
        }
        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in hir.expressions.values(*elements) {
                collect_expression_cast_policies(hir, *element, policies);
            }
        }
        HirExpressionKind::Range { start, end } => {
            collect_expression_cast_policies(hir, *start, policies);
            collect_expression_cast_policies(hir, *end, policies);
        }
        HirExpressionKind::TupleGet { tuple, .. } => {
            collect_expression_cast_policies(hir, *tuple, policies);
        }
        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result } => {
            collect_expression_cast_policies(hir, *result, policies);
        }
        HirExpressionKind::Cast { source, policy } => {
            policies.insert(*policy);
            collect_expression_cast_policies(hir, *source, policies);
        }
        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in hir.expressions.variant_fields(*fields) {
                collect_expression_cast_policies(hir, field.value, policies);
            }
        }
        HirExpressionKind::VariantPayloadGet { source, .. } => {
            collect_expression_cast_policies(hir, *source, policies);
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in hir.expressions.map_entries(*entries) {
                collect_expression_cast_policies(hir, entry.key, policies);
                collect_expression_cast_policies(hir, entry.value, policies);
            }
        }
        HirExpressionKind::Int(_)
        | HirExpressionKind::Uint(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::FixedScalar(_)
        | HirExpressionKind::Number(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => {}
        HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
            collect_place_cast_policies(hir, *place, policies);
        }
    }
}

fn collect_place_cast_policies(
    hir: &HirModule,
    place: HirPlace,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    for projection in hir.expressions.projections(place.projections) {
        if let HirProjection::Index(index) = projection {
            collect_expression_cast_policies(hir, *index, policies);
        }
    }
}

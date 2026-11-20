//! JS emitter — lowers HIR into executable JavaScript.
//!
//! WHAT: converts HIR control flow/expressions into executable JS and symbol maps.
//! WHY: JS is the stable near-term backend and needs deterministic lowering output.

use crate::backends::js::JsModule;
use crate::backends::js::js_statement::NumericDelivery;
use crate::backends::js::numeric_carrier::{JsFastStoreConversion, JsNumericOperationLowering};
use crate::backends::js::runtime::NumericRuntimeHelperUsage;
use crate::backends::js::{ENTRY_FAILURE_NOTICE, JsFunctionEmissionPolicy, JsLoweringConfig};
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, compiler_errors::InfrastructureFailure,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, HirNodeId, HirValueId, LocalId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{HirNumericOp, HirNumericOperands};
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
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
) -> Result<JsModule, InfrastructureFailure> {
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

/// One function chosen for emission, paired with the live block closure computed for it once.
///
/// WHAT: the function selected for emission plus the block set that map discovery, numeric-helper
///       discovery, cast-policy discovery and dispatcher emission all read.
/// WHY: each scan previously rebuilt and sorted the same closure per function, reclassifying every
///      numeric Invoke. One precomputed closure per function keeps a single owner for the pass and
///      keeps the set travelling with the function it describes.
struct FunctionEmission<'hir> {
    function: &'hir HirFunction,
    /// Blocks reachable through an edge that can execute. Local declarations use the separate
    /// unpruned closure.
    live_blocks: Vec<BlockId>,
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
        Self {
            out: String::new(),
            hir,
            numeric_proofs,
            string_table,
            path_table,
            config,
            type_environment,
            indent: 0,
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

    fn lower_module(mut self) -> Result<JsModule, InfrastructureFailure> {
        if let JsFunctionEmissionPolicy::Selected(selection) = &self.config.function_emission_policy
        {
            selection.validate_for_hir(self.hir)?;
        }
        self.build_symbol_maps()?;

        let functions = self.functions_to_emit()?;
        let start_is_fallible = functions.iter().any(|emission| {
            let function = emission.function;
            Some(function.id) == self.hir.start_function && self.function_is_fallible(function)
        });
        let emitted_code_uses_maps = self.emitted_functions_use_maps(&functions)?;
        let mut emitted_code_uses_numeric_helpers =
            self.emitted_functions_use_numeric_helpers(&functions)?;
        self.collect_used_cast_policies(&functions)?;
        for policy in &self.used_cast_policies {
            if let BuiltinCastPolicyId::NumericToString(scalar) = *policy
                && let Some(precision) = scalar.binary_float_precision()
            {
                emitted_code_uses_numeric_helpers.require_float_formatter(precision);
                // The demanded float-to-string cast helper consumes the formatter's carrier
                // through the shared numeric trap helper.
                emitted_code_uses_numeric_helpers.numeric_trap = true;
            }
        }
        self.emit_runtime_prelude(emitted_code_uses_maps, emitted_code_uses_numeric_helpers);

        for (index, emission) in functions.into_iter().enumerate() {
            if index > 0 {
                self.emit_line("");
            }

            self.emit_function(emission.function, &emission.live_blocks)?;
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
                return Err(InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
                    format!(
                        "JavaScript backend: start function {:?} has no generated JS name",
                        start_function
                    ),
                ));
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

    /// Selects the emitted functions and computes each live block closure once.
    ///
    /// WHAT: pairs every function the emission policy includes with the live reachable set that
    ///       every discovery scan and dispatcher body reads.
    /// WHY: the closure is identical for all consumers; computing it here keeps one owner without
    ///      making each scan build its own set or reclassify numeric Invokes.
    fn functions_to_emit(&self) -> Result<Vec<FunctionEmission<'hir>>, InfrastructureFailure> {
        let mut functions = Vec::with_capacity(self.hir.functions.len());
        for function in self
            .hir
            .functions
            .iter()
            .filter(|function| self.config.function_emission_policy.includes(function.id))
        {
            functions.push(FunctionEmission {
                function,
                live_blocks: self.collect_reachable_blocks(function.entry)?,
            });
        }
        functions.sort_by_key(|emission| emission.function.id.0);

        Ok(functions)
    }

    /// Records the carrier families needed by reachable numeric statements and Invokes in emitted
    /// functions.
    ///
    /// WHAT: scans the emitted function/block subset and classifies each reachable operation
    ///       through the same owner as statement lowering, so a fast, total scalar or proven
    ///       integer operation demands only what it actually emits.
    /// WHY: a BigInt operator must not inherit a Number helper merely because both domains are
    ///      integers, and an Invoke reads a checked carrier directly instead of demanding the
    ///      statement trap helper. A checked statement adds that trap demand here, mirroring
    ///      `emit_numeric_carrier_assignment`.
    fn emitted_functions_use_numeric_helpers(
        &self,
        functions: &[FunctionEmission<'hir>],
    ) -> Result<NumericRuntimeHelperUsage, InfrastructureFailure> {
        let mut usage = NumericRuntimeHelperUsage::default();

        for emission in functions {
            for block_id in &emission.live_blocks {
                let block = self.block_by_id(*block_id)?;
                for statement in &block.statements {
                    match &statement.kind {
                        HirStatementKind::NumericOp {
                            op, failure_mode, ..
                        } => {
                            let lowering = self.collect_numeric_operation_helpers(
                                *op,
                                statement.id,
                                NumericDelivery::Statement(*failure_mode),
                                &mut usage,
                            )?;
                            // Only a non-scalar statement consumes the recoverable carrier
                            // through `__moth_numeric_trap`; a scalar fast, total, discharged or
                            // proven operation never demands it.
                            if !lowering.is_scalar() {
                                usage.numeric_trap = true;
                            }
                        }
                        HirStatementKind::FastStoreConversion { conversion, .. } => {
                            JsFastStoreConversion::classify(*conversion)?
                                .require_helpers(&mut usage);
                        }
                        HirStatementKind::FormatFloat { source, .. } => {
                            // The formatted source's own exact precision selects the formatter the
                            // prelude must carry, exactly like statement lowering.
                            let (_scalar, precision) =
                                self.float_source_scalar(*source, "FormatFloat");
                            usage.require_float_formatter(precision);
                            usage.numeric_trap = true;
                        }
                        HirStatementKind::ValidateFloat { .. } => {
                            usage.validate_float = true;
                            usage.numeric_trap = true;
                        }
                        // A checked loop guard consumes the trap carrier; a fast guard throws its
                        // canonical cause directly and demands no helper.
                        HirStatementKind::LoopStepFailure { semantics, .. }
                            if !semantics.is_fast() =>
                        {
                            usage.numeric_trap = true;
                        }
                        _ => {}
                    }
                }
                if let HirTerminator::Invoke { id, operation, .. } = &block.terminator {
                    match operation {
                        HirFallibleOperation::NumericOp { op, .. } => {
                            self.collect_numeric_operation_helpers(
                                *op,
                                *id,
                                NumericDelivery::Invoke,
                                &mut usage,
                            )?;
                        }
                        HirFallibleOperation::FormatFloat { source } => {
                            let (_, precision) = self.float_source_scalar(*source, "FormatFloat");
                            usage.require_float_formatter(precision);
                        }
                        HirFallibleOperation::ValidateFloat { .. } => usage.validate_float = true,
                        // A loop-step Invoke reads the recoverable carrier directly, and a fast
                        // guard throws without a carrier, so neither demands a runtime helper.
                        HirFallibleOperation::LoopStepFailure { .. }
                        | HirFallibleOperation::Call { .. }
                        | HirFallibleOperation::CastOp { .. }
                        | HirFallibleOperation::MapOp { .. } => {}
                    }
                }
            }
        }

        Ok(usage)
    }

    /// Classifies one reachable numeric operation and records its helper demand.
    ///
    /// WHAT: returns the shared lowering so the caller can add statement-delivery trap demand.
    /// WHY: the demand scan must apply the same fast/scalar/proven/checked decision as lowering;
    ///      a second operator or proof test here could demand a helper the operation never calls.
    fn collect_numeric_operation_helpers(
        &self,
        op: HirNumericOp,
        id: HirNodeId,
        delivery: NumericDelivery,
        usage: &mut NumericRuntimeHelperUsage,
    ) -> Result<JsNumericOperationLowering, InfrastructureFailure> {
        let lowering = self.classify_numeric_operation(op, id, delivery)?;
        lowering.require_helpers(usage);

        Ok(lowering)
    }

    /// Returns true when any emitted reachable JS body can construct, store, copy, display, or
    /// operate on a Moth map.
    ///
    /// WHAT: scans the same function/block subset that JS lowering will emit.
    /// WHY: map helpers are new runtime surface. Emitting them only for map-using programs avoids
    /// changing existing golden artifacts while still making map copy/string fallback paths safe.
    fn emitted_functions_use_maps(
        &self,
        functions: &[FunctionEmission<'hir>],
    ) -> Result<bool, InfrastructureFailure> {
        for emission in functions {
            let return_contract = emission.function.return_contract.ordinary();
            let returns_a_map = return_contract.is_some_and(|returns| {
                returns
                    .success()
                    .iter()
                    .copied()
                    .chain(returns.error())
                    .any(|ty| self.type_environment.is_map_type(ty))
            });
            if returns_a_map {
                return Ok(true);
            }

            for block_id in &emission.live_blocks {
                let block = self.block_by_id(*block_id)?;
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

            HirStatementKind::FormatFloat { source, .. }
            | HirStatementKind::ValidateFloat { source, .. } => self.expression_uses_maps(*source),

            HirStatementKind::MapOp { .. } => true,

            HirStatementKind::NumericOp { operands, .. } => {
                self.numeric_operands_use_maps(operands)
            }

            HirStatementKind::FastStoreConversion { source, .. } => {
                self.expression_uses_maps(*source)
            }

            HirStatementKind::RangeCandidate {
                current, step, end, ..
            } => {
                self.expression_uses_maps(*current)
                    || self.expression_uses_maps(*step)
                    || self.expression_uses_maps(*end)
            }
            HirStatementKind::LoopStepFailure { .. } => false,
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
        functions: &[FunctionEmission<'hir>],
    ) -> Result<(), InfrastructureFailure> {
        self.used_cast_policies.clear();

        for emission in functions {
            for block_id in &emission.live_blocks {
                let block = self.block_by_id(*block_id)?;

                for statement in &block.statements {
                    collect_statement_cast_policies(
                        self.hir,
                        &statement.kind,
                        &mut self.used_cast_policies,
                    );
                }

                // Proven Invoke narrowings need only their source expression's cast helpers.
                // Retained uses of the same policy elsewhere still demand its runtime helper.
                match &block.terminator {
                    HirTerminator::Invoke {
                        id,
                        operation: HirFallibleOperation::CastOp { policy, source },
                        ..
                    } if self.integer_narrowing_is_proven(*id, *policy) => {
                        collect_expression_cast_policies(
                            self.hir,
                            *source,
                            &mut self.used_cast_policies,
                        );
                    }
                    terminator => collect_terminator_cast_policies(
                        self.hir,
                        terminator,
                        &mut self.used_cast_policies,
                    ),
                }
            }
        }

        Ok(())
    }

    fn terminator_uses_maps(&self, terminator: &HirTerminator) -> bool {
        match terminator {
            HirTerminator::If { condition, .. } => self.expression_uses_maps(*condition),

            HirTerminator::Invoke { operation, .. } => match operation {
                HirFallibleOperation::MapOp { .. } => true,
                HirFallibleOperation::Call { args, .. } => self
                    .hir
                    .expressions
                    .values(*args)
                    .iter()
                    .any(|value| self.expression_uses_maps(*value)),
                HirFallibleOperation::CastOp { source, .. }
                | HirFallibleOperation::FormatFloat { source }
                | HirFallibleOperation::ValidateFloat { source } => {
                    self.expression_uses_maps(*source)
                }
                HirFallibleOperation::NumericOp { operands, .. } => {
                    self.numeric_operands_use_maps(operands)
                }
                HirFallibleOperation::LoopStepFailure { .. } => false,
            },

            HirTerminator::Match { scrutinee, arms } => {
                self.expression_uses_maps(*scrutinee)
                    || arms.iter().any(|arm| {
                        arm.guard
                            .is_some_and(|guard| self.expression_uses_maps(guard))
                            || self.pattern_uses_maps(&arm.pattern)
                    })
            }

            HirTerminator::Return(values) => self
                .hir
                .expressions
                .values(*values)
                .iter()
                .any(|value| self.expression_uses_maps(*value)),
            HirTerminator::ReturnError(value) => self.expression_uses_maps(*value),

            HirTerminator::AssertFailure { message, .. } => self.expression_uses_maps(*message),

            HirTerminator::NeverCall { args, .. } => self
                .hir
                .expressions
                .values(*args)
                .iter()
                .any(|value| self.expression_uses_maps(*value)),

            HirTerminator::Jump { .. }
            | HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::Uninitialized
            | HirTerminator::RuntimeFailure { .. } => false,
        }
    }

    fn pattern_uses_maps(&self, pattern: &HirPattern) -> bool {
        pattern
            .value()
            .is_some_and(|value| self.expression_uses_maps(value))
    }

    fn expression_uses_maps(&self, expression_id: HirValueId) -> bool {
        // WHAT: scans in source pre-order with an explicit work list of pending siblings.
        // WHY: a single-child chain must not add a native frame per expression level.
        let mut pending = Vec::new();
        let mut next = Some(expression_id);
        while let Some(current) = next.take().or_else(|| pending.pop()) {
            let expression = self.hir.expressions.expression(current);
            // A map literal is itself the map use, so its entries are never scanned separately.
            if self.type_environment.is_map_type(expression.ty)
                || matches!(expression.kind, HirExpressionKind::MapLiteral(_))
            {
                return true;
            }

            next = self.hir.expressions.push_child_values_in_source_order(
                current,
                &mut pending,
                |child| child,
            );
        }
        false
    }

    fn place_uses_maps(&self, place: HirPlace) -> bool {
        self.hir
            .expressions
            .place_index_values(place)
            .any(|index| self.expression_uses_maps(index))
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
        HirStatementKind::FastStoreConversion { source, .. } => {
            collect_expression_cast_policies(hir, *source, policies);
        }
        HirStatementKind::RangeCandidate {
            current, step, end, ..
        } => {
            collect_expression_cast_policies(hir, *current, policies);
            collect_expression_cast_policies(hir, *step, policies);
            collect_expression_cast_policies(hir, *end, policies);
        }
        HirStatementKind::LoopStepFailure { .. } => {}
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
        HirTerminator::Invoke { operation, .. } => match operation {
            HirFallibleOperation::Call { args, .. } => {
                for argument in hir.expressions.values(*args) {
                    collect_expression_cast_policies(hir, *argument, policies);
                }
            }
            HirFallibleOperation::MapOp { receiver, args, .. } => {
                collect_expression_cast_policies(hir, *receiver, policies);
                for argument in hir.expressions.values(*args) {
                    collect_expression_cast_policies(hir, *argument, policies);
                }
            }
            HirFallibleOperation::CastOp { policy, source } => {
                policies.insert(*policy);
                collect_expression_cast_policies(hir, *source, policies);
            }
            HirFallibleOperation::FormatFloat { source }
            | HirFallibleOperation::ValidateFloat { source } => {
                collect_expression_cast_policies(hir, *source, policies);
            }
            HirFallibleOperation::NumericOp { operands, .. } => match operands {
                HirNumericOperands::Unary { operand } => {
                    collect_expression_cast_policies(hir, *operand, policies);
                }
                HirNumericOperands::Binary { left, right } => {
                    collect_expression_cast_policies(hir, *left, policies);
                    collect_expression_cast_policies(hir, *right, policies);
                }
            },
            HirFallibleOperation::LoopStepFailure { .. } => {}
        },
        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_cast_policies(hir, *scrutinee, policies);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    collect_expression_cast_policies(hir, guard, policies);
                }
                collect_pattern_cast_policies(hir, &arm.pattern, policies);
            }
        }
        HirTerminator::Return(values) => {
            for value in hir.expressions.values(*values) {
                collect_expression_cast_policies(hir, *value, policies);
            }
        }
        HirTerminator::ReturnError(value) => {
            collect_expression_cast_policies(hir, *value, policies)
        }
        HirTerminator::AssertFailure { message, .. } => {
            collect_expression_cast_policies(hir, *message, policies)
        }
        HirTerminator::NeverCall { args, .. } => {
            for value in hir.expressions.values(*args) {
                collect_expression_cast_policies(hir, *value, policies);
            }
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
    if let Some(value) = pattern.value() {
        collect_expression_cast_policies(hir, value, policies);
    }
}

fn collect_expression_cast_policies(
    hir: &HirModule,
    expression_id: HirValueId,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    // WHAT: scans in source pre-order with an explicit work list of pending siblings.
    // WHY: a single-child chain must not add a native frame per expression level.
    let mut pending = Vec::new();
    let mut next = Some(expression_id);
    while let Some(current) = next.take().or_else(|| pending.pop()) {
        if let HirExpressionKind::Cast { policy, .. } = &hir.expressions.expression(current).kind {
            policies.insert(*policy);
        }

        next = hir
            .expressions
            .push_child_values_in_source_order(current, &mut pending, |child| child);
    }
}

fn collect_place_cast_policies(
    hir: &HirModule,
    place: HirPlace,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    for index in hir.expressions.place_index_values(place) {
        collect_expression_cast_policies(hir, index, policies);
    }
}

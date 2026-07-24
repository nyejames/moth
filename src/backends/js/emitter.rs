//! JS emitter — lowers HIR into executable JavaScript.
//!
//! WHAT: converts HIR control flow/expressions into executable JS and symbol maps.
//! WHY: JS is the stable near-term backend and needs deterministic lowering output.

use crate::backends::js::JsModule;
use crate::backends::js::runtime::NumericRuntimeHelperUsage;
use crate::backends::js::{JsFunctionEmissionPolicy, JsLoweringConfig};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FieldId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::reachability::collect_reachability_from_start;
use crate::compiler_frontend::hir::reactivity::ReactiveSourceId;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::{HashMap, HashSet};

/// Lower one validated HIR module into JavaScript source.
pub fn lower_hir_to_js(
    hir: &HirModule,
    borrow_analysis: &BorrowCheckReport,
    string_table: &StringTable,
    config: JsLoweringConfig,
    type_environment: &TypeEnvironment,
) -> Result<JsModule, CompilerError> {
    let mut emitter = JsEmitter::new(hir, borrow_analysis, string_table, config, type_environment);
    emitter.lower_module()
}

pub(crate) struct JsEmitter<'hir> {
    pub(crate) hir: &'hir HirModule,
    pub(crate) borrow_analysis: &'hir BorrowCheckReport,
    pub(crate) string_table: &'hir StringTable,
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
    /// Builtin cast policies referenced while lowering emitted JS functions.
    /// Used to conditionally emit the matching runtime helpers.
    pub(crate) used_cast_policies: HashSet<BuiltinCastPolicyId>,
    /// Whether emitted reachable JS uses reactive sources.
    /// Used to conditionally emit the reactive binding and scheduler helpers.
    pub(crate) used_reactive_sources: bool,
    /// Whether emitted reachable JS uses reactive template values.
    /// Used to conditionally emit the template-string runtime helpers.
    pub(crate) used_reactive_templates: bool,
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn new(
        hir: &'hir HirModule,
        borrow_analysis: &'hir BorrowCheckReport,
        string_table: &'hir StringTable,
        config: JsLoweringConfig,
        type_environment: &'hir TypeEnvironment,
    ) -> Self {
        let blocks_by_id = hir
            .blocks
            .iter()
            .map(|block| (block.id, block))
            .collect::<HashMap<_, _>>();

        Self {
            hir,
            borrow_analysis,
            string_table,
            config,
            type_environment,
            out: String::new(),
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
            used_cast_policies: HashSet::new(),
            used_reactive_sources: false,
            used_reactive_templates: false,
        }
    }

    fn lower_module(&mut self) -> Result<JsModule, CompilerError> {
        self.build_symbol_maps();

        let functions = self.functions_to_emit()?;
        let emitted_code_uses_maps = self.emitted_functions_use_maps(&functions)?;
        let emitted_code_uses_numeric_helpers =
            self.emitted_functions_use_numeric_helpers(&functions)?;
        self.emitted_functions_use_reactivity(&functions)?;
        self.collect_used_cast_policies(&functions)?;
        self.emit_runtime_prelude(
            emitted_code_uses_maps,
            emitted_code_uses_numeric_helpers,
            self.used_reactive_sources,
            self.used_reactive_templates,
        );

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

        if self.config.auto_invoke_start {
            let Some(start_name) = self
                .function_name_by_id
                .get(&self.hir.start_function)
                .cloned()
            else {
                return Err(CompilerError::compiler_error(format!(
                    "JavaScript backend: start function {:?} has no generated JS name",
                    self.hir.start_function
                )));
            };

            if !self.out.is_empty() {
                self.emit_line("");
            }

            self.emit_line(&format!("{start_name}();"));
        }

        Ok(JsModule {
            source: self.out.clone(),
            function_name_by_id: self.function_name_by_id.clone(),
            referenced_external_functions: self.referenced_external_functions.clone(),
        })
    }

    fn functions_to_emit(&self) -> Result<Vec<&'hir HirFunction>, CompilerError> {
        let reachable_functions = match self.config.function_emission_policy {
            JsFunctionEmissionPolicy::AllFunctions => None,
            JsFunctionEmissionPolicy::ReachableFromStart => {
                Some(self.collect_js_reachable_functions()?)
            }
        };

        let mut functions = self
            .hir
            .functions
            .iter()
            .filter(|function| {
                let Some(reachable) = reachable_functions.as_ref() else {
                    return true;
                };

                reachable.contains(&function.id)
            })
            .collect::<Vec<_>>();
        functions.sort_by_key(|function| function.id.0);

        Ok(functions)
    }

    /// Records which checked numeric helper families are needed by emitted reachable JS bodies.
    ///
    /// WHAT: scans the same function/block subset that JS lowering will emit and records whether
    ///      it contains `NumericOp`, `FormatFloat`, or `ValidateFloat` statements.
    /// WHY: the arithmetic helpers and Float formatting/validation helpers share a trap wrapper
    ///      but are otherwise independent. Splitting demand keeps existing arithmetic-only bundles
    ///      from growing unrelated Float helpers.
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
                    match statement.kind {
                        HirStatementKind::NumericOp { .. } => {
                            usage.numeric_ops = true;
                        }
                        HirStatementKind::FormatFloat { .. } => {
                            usage.format_float = true;
                        }
                        HirStatementKind::ValidateFloat { .. } => {
                            usage.validate_float = true;
                        }
                        _ => {}
                    }

                    if statement_uses_float_to_string_cast(&statement.kind) {
                        usage.format_float = true;
                    }

                    if usage.numeric_ops && usage.format_float && usage.validate_float {
                        return Ok(usage);
                    }
                }

                if terminator_uses_float_to_string_cast(&block.terminator) {
                    usage.format_float = true;
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
            HirStatementKind::Assign { target: _, value }
            | HirStatementKind::Expr(value)
            | HirStatementKind::PushRuntimeFragment { value, .. } => {
                self.expression_uses_maps(value)
            }

            HirStatementKind::Call { args, .. } => args
                .iter()
                .any(|argument| self.expression_uses_maps(argument)),

            HirStatementKind::CastOp { source, .. } => self.expression_uses_maps(source),

            HirStatementKind::FormatFloat { source, .. }
            | HirStatementKind::ValidateFloat { source, .. } => self.expression_uses_maps(source),

            HirStatementKind::MapOp { .. } => true,

            HirStatementKind::NumericOp { operands, .. } => {
                self.numeric_operands_use_maps(operands)
            }

            HirStatementKind::Drop(_) => false,
        }
    }

    fn numeric_operands_use_maps(&self, operands: &HirNumericOperands) -> bool {
        match operands {
            HirNumericOperands::Unary { operand } => self.expression_uses_maps(operand),
            HirNumericOperands::Binary { left, right } => {
                self.expression_uses_maps(left) || self.expression_uses_maps(right)
            }
        }
    }

    /// Scans the emitted reachable function subset to discover which reactivity runtime helpers are
    /// needed.
    ///
    /// WHAT: sets `used_reactive_sources` when emitted code declares a reactive source or has a
    /// borrow-analysis invalidation fact; sets `used_reactive_templates` when any emitted
    /// expression is a reactive template value.
    /// WHY: reactivity helpers add runtime surface and globals; emitting them only for the features
    ///      actually used keeps non-reactive bundles unchanged and avoids dragging template-string
    ///      helpers into source-only reactive programs.
    fn emitted_functions_use_reactivity(
        &mut self,
        functions: &[&'hir HirFunction],
    ) -> Result<(), CompilerError> {
        self.used_reactive_sources = self.emitted_functions_use_reactive_sources(functions)?;
        self.used_reactive_templates = false;

        for function in functions {
            let reachable_blocks = self.collect_reachable_blocks(function.entry)?;
            for block_id in reachable_blocks {
                let block = self.block_by_id(block_id)?;

                for statement in &block.statements {
                    self.record_statement_reactivity(statement)?;
                }

                self.record_terminator_reactivity(&block.terminator)?;
            }
        }

        if self.used_reactive_templates {
            self.used_reactive_sources = true;
        }

        Ok(())
    }

    fn emitted_functions_use_reactive_sources(
        &self,
        functions: &[&'hir HirFunction],
    ) -> Result<bool, CompilerError> {
        for function in functions {
            let reachable_blocks = self.collect_reachable_blocks(function.entry)?;
            for block_id in reachable_blocks {
                let block = self.block_by_id(block_id)?;

                if block
                    .locals
                    .iter()
                    .any(|local| self.local_is_reactive_source(local.id))
                {
                    return Ok(true);
                }

                if block.statements.iter().any(|statement| {
                    self.borrow_analysis
                        .analysis
                        .reactive_invalidations
                        .contains_key(&statement.id)
                }) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub(crate) fn reactive_source_id_for_local(
        &self,
        local_id: LocalId,
    ) -> Option<ReactiveSourceId> {
        self.hir.side_table.reactive_source_id_for_local(local_id)
    }

    pub(crate) fn local_is_reactive_source(&self, local_id: LocalId) -> bool {
        self.reactive_source_id_for_local(local_id).is_some()
    }

    fn record_statement_reactivity(
        &mut self,
        statement: &crate::compiler_frontend::hir::statements::HirStatement,
    ) -> Result<(), CompilerError> {
        if self
            .borrow_analysis
            .analysis
            .reactive_invalidations
            .contains_key(&statement.id)
        {
            self.used_reactive_sources = true;
        }

        match &statement.kind {
            HirStatementKind::Assign { value, .. } => {
                self.record_expression_reactivity(value)?;
            }

            HirStatementKind::Call { args, .. } => {
                for argument in args {
                    self.record_expression_reactivity(argument)?;
                }
            }

            HirStatementKind::CastOp { source, .. } => {
                self.record_expression_reactivity(source)?;
            }

            HirStatementKind::FormatFloat { source, .. }
            | HirStatementKind::ValidateFloat { source, .. } => {
                self.record_expression_reactivity(source)?;
            }

            HirStatementKind::Expr(value) | HirStatementKind::PushRuntimeFragment { value, .. } => {
                self.record_expression_reactivity(value)?;
            }

            HirStatementKind::MapOp { receiver, args, .. } => {
                self.record_expression_reactivity(receiver)?;
                for argument in args {
                    self.record_expression_reactivity(argument)?;
                }
            }

            HirStatementKind::NumericOp { operands, .. } => match operands {
                HirNumericOperands::Unary { operand } => {
                    self.record_expression_reactivity(operand)?;
                }
                HirNumericOperands::Binary { left, right } => {
                    self.record_expression_reactivity(left)?;
                    self.record_expression_reactivity(right)?;
                }
            },

            HirStatementKind::Drop(_) => {}
        }

        Ok(())
    }

    fn record_terminator_reactivity(
        &mut self,
        terminator: &HirTerminator,
    ) -> Result<(), CompilerError> {
        match terminator {
            HirTerminator::If { condition, .. } => {
                self.record_expression_reactivity(condition)?;
            }

            HirTerminator::FallibleBranch { result, .. } => {
                self.record_expression_reactivity(result)?;
            }

            HirTerminator::Match { scrutinee, arms } => {
                self.record_expression_reactivity(scrutinee)?;
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.record_expression_reactivity(guard)?;
                    }
                    self.record_pattern_reactivity(&arm.pattern)?;
                }
            }

            HirTerminator::Return(value)
            | HirTerminator::ReturnSuccess(value)
            | HirTerminator::ReturnError(value) => {
                self.record_expression_reactivity(value)?;
            }

            HirTerminator::Jump { .. }
            | HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::Uninitialized
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::AssertFailure { .. } => {}
        }

        Ok(())
    }

    fn record_pattern_reactivity(&mut self, pattern: &HirPattern) -> Result<(), CompilerError> {
        match pattern {
            HirPattern::Literal(value)
            | HirPattern::OptionValue { value }
            | HirPattern::OptionRelational { value, .. }
            | HirPattern::Relational { value, .. } => {
                self.record_expression_reactivity(value)?;
            }

            HirPattern::OptionNone
            | HirPattern::OptionPresent
            | HirPattern::Wildcard
            | HirPattern::ChoiceVariant { .. }
            | HirPattern::Capture => {}
        }

        Ok(())
    }

    fn record_expression_reactivity(
        &mut self,
        expression: &HirExpression,
    ) -> Result<(), CompilerError> {
        if self.value_is_reactive_template(expression.id) {
            self.used_reactive_templates = true;
        }

        match &expression.kind {
            HirExpressionKind::BinOp { left, right, .. } => {
                self.record_expression_reactivity(left)?;
                self.record_expression_reactivity(right)?;
            }

            HirExpressionKind::UnaryOp { operand, .. } => {
                self.record_expression_reactivity(operand)?;
            }

            HirExpressionKind::StructConstruct { fields, .. } => {
                for (_, field_value) in fields {
                    self.record_expression_reactivity(field_value)?;
                }
            }

            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => {
                for element in elements {
                    self.record_expression_reactivity(element)?;
                }
            }

            HirExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    self.record_expression_reactivity(&entry.key)?;
                    self.record_expression_reactivity(&entry.value)?;
                }
            }

            HirExpressionKind::Range { start, end } => {
                self.record_expression_reactivity(start)?;
                self.record_expression_reactivity(end)?;
            }

            HirExpressionKind::TupleGet { tuple, .. } => {
                self.record_expression_reactivity(tuple)?;
            }

            HirExpressionKind::FallibleUnwrapSuccess { result }
            | HirExpressionKind::FallibleUnwrapError { result } => {
                self.record_expression_reactivity(result)?;
            }

            HirExpressionKind::Cast { source, .. } => {
                self.record_expression_reactivity(source)?;
            }

            HirExpressionKind::VariantConstruct { fields, .. } => {
                for field in fields {
                    self.record_expression_reactivity(&field.value)?;
                }
            }

            HirExpressionKind::VariantPayloadGet { source, .. } => {
                self.record_expression_reactivity(source)?;
            }

            HirExpressionKind::Load(_)
            | HirExpressionKind::Copy(_)
            | HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => {}
        }

        Ok(())
    }

    pub(crate) fn value_is_reactive_template(&self, value_id: HirValueId) -> bool {
        let Some(template) = self.hir.side_table.reactive_template_for_value(value_id) else {
            return false;
        };

        if !template.dependencies.is_empty() {
            return true;
        }

        // Ordinary `String` parameters can carry placeholder template metadata so reactive
        // template values preserve dependencies when passed through helper functions. In a module
        // with no emitted reactive source, those placeholders cannot resolve to a live template
        // object, so non-reactive programs should keep their plain-string lowering and avoid
        // pulling in the template runtime helpers.
        self.used_reactive_sources && !template.template_value_parameters.is_empty()
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
                    collect_statement_cast_policies(&statement.kind, &mut self.used_cast_policies);
                }

                collect_terminator_cast_policies(&block.terminator, &mut self.used_cast_policies);
            }
        }

        Ok(())
    }

    fn terminator_uses_maps(&self, terminator: &HirTerminator) -> bool {
        match terminator {
            HirTerminator::If { condition, .. } => self.expression_uses_maps(condition),

            HirTerminator::FallibleBranch { result, .. } => self.expression_uses_maps(result),

            HirTerminator::Match { scrutinee, arms } => {
                self.expression_uses_maps(scrutinee)
                    || arms.iter().any(|arm| {
                        arm.guard
                            .as_ref()
                            .is_some_and(|guard| self.expression_uses_maps(guard))
                            || self.pattern_uses_maps(&arm.pattern)
                    })
            }

            HirTerminator::Return(value)
            | HirTerminator::ReturnSuccess(value)
            | HirTerminator::ReturnError(value) => self.expression_uses_maps(value),

            HirTerminator::Jump { .. }
            | HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::Uninitialized
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::AssertFailure { .. } => false,
        }
    }

    fn pattern_uses_maps(&self, pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Literal(value)
            | HirPattern::OptionValue { value }
            | HirPattern::OptionRelational { value, .. }
            | HirPattern::Relational { value, .. } => self.expression_uses_maps(value),

            HirPattern::OptionNone
            | HirPattern::OptionPresent
            | HirPattern::Wildcard
            | HirPattern::ChoiceVariant { .. }
            | HirPattern::Capture => false,
        }
    }

    fn expression_uses_maps(&self, expression: &HirExpression) -> bool {
        if self.type_environment.is_map_type(expression.ty) {
            return true;
        }

        match &expression.kind {
            HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => false,

            HirExpressionKind::BinOp { left, right, .. } => {
                self.expression_uses_maps(left) || self.expression_uses_maps(right)
            }

            HirExpressionKind::UnaryOp { operand, .. } => self.expression_uses_maps(operand),

            HirExpressionKind::StructConstruct { fields, .. } => fields
                .iter()
                .any(|(_, field_value)| self.expression_uses_maps(field_value)),

            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => elements
                .iter()
                .any(|element| self.expression_uses_maps(element)),

            HirExpressionKind::MapLiteral(_) => true,

            HirExpressionKind::Range { start, end } => {
                self.expression_uses_maps(start) || self.expression_uses_maps(end)
            }

            HirExpressionKind::TupleGet { tuple, .. } => self.expression_uses_maps(tuple),

            HirExpressionKind::FallibleUnwrapSuccess { result }
            | HirExpressionKind::FallibleUnwrapError { result } => {
                self.expression_uses_maps(result)
            }

            HirExpressionKind::Cast { source, .. } => self.expression_uses_maps(source),

            HirExpressionKind::VariantConstruct { fields, .. } => fields
                .iter()
                .any(|field| self.expression_uses_maps(&field.value)),

            HirExpressionKind::VariantPayloadGet { source, .. } => {
                self.expression_uses_maps(source)
            }

            HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => false,
        }
    }

    fn collect_js_reachable_functions(
        &self,
    ) -> Result<rustc_hash::FxHashSet<FunctionId>, CompilerError> {
        let reachability = collect_reachability_from_start(self.hir)?;
        Ok(reachability.reachable_functions)
    }
}

fn collect_statement_cast_policies(
    statement: &HirStatementKind,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match statement {
        HirStatementKind::Assign { value, .. }
        | HirStatementKind::Expr(value)
        | HirStatementKind::PushRuntimeFragment { value, .. } => {
            collect_expression_cast_policies(value, policies);
        }

        HirStatementKind::Call { args, .. } => {
            for argument in args {
                collect_expression_cast_policies(argument, policies);
            }
        }

        HirStatementKind::CastOp { policy, source, .. } => {
            policies.insert(*policy);
            collect_expression_cast_policies(source, policies);
        }

        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            collect_expression_cast_policies(source, policies);
        }

        HirStatementKind::MapOp { receiver, args, .. } => {
            collect_expression_cast_policies(receiver, policies);
            for argument in args {
                collect_expression_cast_policies(argument, policies);
            }
        }

        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => {
                collect_expression_cast_policies(operand, policies);
            }
            HirNumericOperands::Binary { left, right } => {
                collect_expression_cast_policies(left, policies);
                collect_expression_cast_policies(right, policies);
            }
        },

        HirStatementKind::Drop(_) => {}
    }
}

fn statement_uses_float_to_string_cast(statement: &HirStatementKind) -> bool {
    let mut policies = HashSet::new();
    collect_statement_cast_policies(statement, &mut policies);
    policies.contains(&BuiltinCastPolicyId::FloatToString)
}

fn collect_terminator_cast_policies(
    terminator: &HirTerminator,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match terminator {
        HirTerminator::If { condition, .. } => {
            collect_expression_cast_policies(condition, policies)
        }

        HirTerminator::FallibleBranch { result, .. } => {
            collect_expression_cast_policies(result, policies);
        }

        HirTerminator::Match { scrutinee, arms } => {
            collect_expression_cast_policies(scrutinee, policies);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_expression_cast_policies(guard, policies);
                }
                collect_pattern_cast_policies(&arm.pattern, policies);
            }
        }

        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => collect_expression_cast_policies(value, policies),

        HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::AssertFailure { .. } => {}
    }
}

fn terminator_uses_float_to_string_cast(terminator: &HirTerminator) -> bool {
    let mut policies = HashSet::new();
    collect_terminator_cast_policies(terminator, &mut policies);
    policies.contains(&BuiltinCastPolicyId::FloatToString)
}

fn collect_pattern_cast_policies(
    pattern: &HirPattern,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match pattern {
        HirPattern::Literal(value)
        | HirPattern::OptionValue { value }
        | HirPattern::OptionRelational { value, .. }
        | HirPattern::Relational { value, .. } => collect_expression_cast_policies(value, policies),

        HirPattern::OptionNone
        | HirPattern::OptionPresent
        | HirPattern::Wildcard
        | HirPattern::ChoiceVariant { .. }
        | HirPattern::Capture => {}
    }
}

fn collect_expression_cast_policies(
    expression: &HirExpression,
    policies: &mut HashSet<BuiltinCastPolicyId>,
) {
    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            collect_expression_cast_policies(left, policies);
            collect_expression_cast_policies(right, policies);
        }

        HirExpressionKind::UnaryOp { operand, .. } => {
            collect_expression_cast_policies(operand, policies)
        }

        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in fields {
                collect_expression_cast_policies(value, policies);
            }
        }

        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in elements {
                collect_expression_cast_policies(element, policies);
            }
        }

        HirExpressionKind::Range { start, end } => {
            collect_expression_cast_policies(start, policies);
            collect_expression_cast_policies(end, policies);
        }

        HirExpressionKind::TupleGet { tuple, .. } => {
            collect_expression_cast_policies(tuple, policies);
        }

        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result } => {
            collect_expression_cast_policies(result, policies);
        }

        HirExpressionKind::Cast { source, policy } => {
            policies.insert(*policy);
            collect_expression_cast_policies(source, policies);
        }

        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                collect_expression_cast_policies(&field.value, policies);
            }
        }

        HirExpressionKind::VariantPayloadGet { source, .. } => {
            collect_expression_cast_policies(source, policies);
        }

        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                collect_expression_cast_policies(&entry.key, policies);
                collect_expression_cast_policies(&entry.value, policies);
            }
        }

        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::Load(_)
        | HirExpressionKind::Copy(_) => {}
    }
}

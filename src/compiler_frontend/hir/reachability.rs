//! Backend-neutral HIR reachability analysis.
//!
//! WHAT: walks the explicit HIR call graph and CFG from one or more root functions, reporting
//! reachable user functions, blocks, and stable external function IDs.
//! WHY: build-system and backend phases need one shared view of which runtime calls can execute
//! without re-scanning import syntax or inventing target-specific reachability rules.
//!
//! This is intentionally a syntactic HIR analysis. It does not fold constants, eliminate dead
//! branches, inspect borrow facts, or perform backend lowering.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, HirMapOp};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::reactivity::ReactiveTemplateId;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Reachable HIR surface from the selected root functions.
///
/// WHY: later phases need both the user-code slice and the external package calls that are
/// actually reachable, but ownership of artifact planning stays outside HIR.
#[derive(Clone, Debug, Default)]
pub(crate) struct HirReachability {
    pub(crate) reachable_functions: FxHashSet<FunctionId>,
    pub(crate) reachable_blocks: FxHashSet<BlockId>,
    pub(crate) reachable_external_functions: FxHashSet<ExternalFunctionId>,
    pub(crate) reachable_external_calls: Vec<ReachableExternalCall>,
    pub(crate) reachable_map_uses: Vec<ReachableMapUse>,
    pub(crate) reachable_reactive_templates: Vec<ReachableReactiveTemplateUse>,
    pub(crate) reachable_reactive_sinks: Vec<ReachableReactiveSinkUse>,
    pub(crate) reachable_runtime_casts: Vec<ReachableRuntimeCastUse>,
    pub(crate) reachable_numeric_ops: Vec<ReachableNumericOpUse>,
    pub(crate) reachable_float_statements: Vec<ReachableFloatStatementUse>,
}

/// A reachable map construction or use at the HIR statement or expression that produces it.
///
/// WHY: backend unsupported-feature validation needs to know which map literals and map
///      operations are reachable from entry so it can emit structured diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableMapUse {
    pub(crate) kind: ReachableMapUseKind,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReachableMapUseKind {
    Literal,
    Operation(HirMapOp),
}

/// A reachable external call at the HIR statement that invokes it.
///
/// WHY: backend validation needs the stable function ID for support checks and the exact
/// statement location for user-facing unsupported-backend diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableExternalCall {
    pub(crate) function_id: ExternalFunctionId,
    pub(crate) statement_id: HirNodeId,
    pub(crate) location: SourceLocation,
}

/// A reachable reactive template-backed value.
///
/// WHY: unsupported-backend validation needs to reject reachable reactive runtime features even
/// when they are produced inside helper functions rather than directly pushed into the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableReactiveTemplateUse {
    pub(crate) template_id: ReactiveTemplateId,
    pub(crate) location: SourceLocation,
}

/// A reachable sink that consumes a reactive template-backed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableReactiveSinkUse {
    pub(crate) kind: ReachableReactiveSinkKind,
    pub(crate) template_id: ReactiveTemplateId,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReachableReactiveSinkKind {
    RuntimeFragment,
    ExternalCallArgument {
        function_id: ExternalFunctionId,
        argument_index: usize,
    },
}

/// A reachable compiler-owned builtin runtime cast expression or statement.
///
/// WHY: some backends (currently HTML-Wasm) cannot lower runtime casts yet. Recording the cast
///      site in reachability lets backend feature validation report the first reachable unsupported
///      cast without re-scanning HIR expressions locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableRuntimeCastUse {
    pub(crate) location: SourceLocation,
}

/// A reachable compiler-owned checked numeric operation.
///
/// WHY: backends that do not yet implement checked numeric semantics must reject the reachable HIR
///      operation before lowering instead of failing with a backend-internal error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableNumericOpUse {
    pub(crate) location: SourceLocation,
}

/// A reachable compiler-owned Float formatting or validation statement.
///
/// WHY: backends that do not yet implement Moth Float formatting or external-Float boundary
///      validation must reject the reachable HIR operation before lowering instead of failing with a
///      backend-internal error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableFloatStatementUse {
    pub(crate) kind: ReachableFloatStatementKind,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReachableFloatStatementKind {
    FormatFloat,
    ValidateFloat,
}

pub(crate) struct HirReachabilityInput<'a> {
    pub(crate) hir: &'a HirModule,
    pub(crate) root_functions: Vec<FunctionId>,
}

struct HirReachabilityContext<'a> {
    hir: &'a HirModule,
    function_by_id: FxHashMap<FunctionId, &'a HirFunction>,
    block_by_id: FxHashMap<BlockId, &'a HirBlock>,
    function_worklist: VecDeque<FunctionId>,
    block_worklist: VecDeque<BlockId>,
    reachability: HirReachability,
}

pub(crate) fn collect_reachability_from_start(
    hir: &HirModule,
) -> Result<HirReachability, CompilerError> {
    collect_hir_reachability(HirReachabilityInput {
        hir,
        root_functions: vec![hir.start_function],
    })
}

pub(crate) fn collect_hir_reachability(
    input: HirReachabilityInput<'_>,
) -> Result<HirReachability, CompilerError> {
    let mut context = HirReachabilityContext::new(input.hir)?;

    for root_function in input.root_functions {
        context.enqueue_function(root_function);
    }

    context.collect()
}

impl<'a> HirReachabilityContext<'a> {
    fn new(hir: &'a HirModule) -> Result<Self, CompilerError> {
        let function_by_id = build_function_map(hir)?;
        let block_by_id = build_block_map(hir)?;

        Ok(Self {
            hir,
            function_by_id,
            block_by_id,
            function_worklist: VecDeque::new(),
            block_worklist: VecDeque::new(),
            reachability: HirReachability::default(),
        })
    }

    fn collect(mut self) -> Result<HirReachability, CompilerError> {
        while !self.function_worklist.is_empty() || !self.block_worklist.is_empty() {
            while let Some(function_id) = self.function_worklist.pop_front() {
                self.visit_function(function_id)?;
            }

            while let Some(block_id) = self.block_worklist.pop_front() {
                self.visit_block(block_id)?;
            }
        }

        Ok(self.reachability)
    }

    fn visit_function(&mut self, function_id: FunctionId) -> Result<(), CompilerError> {
        if !self.reachability.reachable_functions.insert(function_id) {
            return Ok(());
        }

        let Some(function) = self.function_by_id.get(&function_id).copied() else {
            return Err(hir_reachability_error(format!(
                "Unknown HIR function id {function_id:?} reached HIR reachability analysis"
            )));
        };

        self.enqueue_block(function.entry);
        Ok(())
    }

    fn visit_block(&mut self, block_id: BlockId) -> Result<(), CompilerError> {
        if !self.reachability.reachable_blocks.insert(block_id) {
            return Ok(());
        }

        let Some(block) = self.block_by_id.get(&block_id).copied() else {
            return Err(hir_reachability_error(format!(
                "Unknown HIR block id {block_id:?} reached HIR reachability analysis"
            )));
        };

        self.visit_block_statements(block);
        self.collect_runtime_feature_uses_from_terminator(block);
        self.enqueue_terminator_successors(&block.terminator)
    }

    fn visit_block_statements(&mut self, block: &HirBlock) {
        // HIR lowering flattens calls into statements; expression trees intentionally do not
        // carry call targets. Keep the reachability boundary here unless HIR gains a call
        // expression variant in a later design.
        for statement in &block.statements {
            self.collect_runtime_feature_uses_from_statement(statement);

            let HirStatementKind::Call { target, .. } = &statement.kind else {
                continue;
            };

            match target {
                CallTarget::UserFunction(function_id) => self.enqueue_function(*function_id),
                CallTarget::ExternalFunction(function_id) => {
                    self.reachability
                        .reachable_external_functions
                        .insert(*function_id);
                    self.reachability
                        .reachable_external_calls
                        .push(ReachableExternalCall {
                            function_id: *function_id,
                            statement_id: statement.id,
                            location: statement.location.clone(),
                        });
                }
            }
        }
    }

    fn collect_runtime_feature_uses_from_statement(&mut self, statement: &HirStatement) {
        match &statement.kind {
            // Expressions and calls: recurse into sub-expressions only.
            HirStatementKind::Assign { value, .. } | HirStatementKind::Expr(value) => {
                self.collect_runtime_feature_uses_from_expression(value, &statement.location);
            }

            HirStatementKind::Call { target, args, .. } => {
                for (argument_index, arg) in args.iter().enumerate() {
                    if let CallTarget::ExternalFunction(function_id) = target {
                        self.collect_reactive_sink_from_expression(
                            ReachableReactiveSinkKind::ExternalCallArgument {
                                function_id: *function_id,
                                argument_index,
                            },
                            arg,
                            &statement.location,
                        );
                    }
                    self.collect_runtime_feature_uses_from_expression(arg, &statement.location);
                }
            }

            HirStatementKind::PushRuntimeFragment { value, .. } => {
                self.collect_reactive_sink_from_expression(
                    ReachableReactiveSinkKind::RuntimeFragment,
                    value,
                    &statement.location,
                );
                self.collect_runtime_feature_uses_from_expression(value, &statement.location);
            }

            // Map operations: record the use, then recurse into receiver and args.
            HirStatementKind::MapOp {
                op, receiver, args, ..
            } => {
                self.reachability.reachable_map_uses.push(ReachableMapUse {
                    kind: ReachableMapUseKind::Operation(*op),
                    location: statement.location.clone(),
                });
                self.collect_runtime_feature_uses_from_expression(receiver, &statement.location);
                for arg in args {
                    self.collect_runtime_feature_uses_from_expression(arg, &statement.location);
                }
            }

            HirStatementKind::Drop(_) => {}

            HirStatementKind::NumericOp { operands, .. } => {
                self.reachability
                    .reachable_numeric_ops
                    .push(ReachableNumericOpUse {
                        location: statement.location.clone(),
                    });

                match operands {
                    HirNumericOperands::Unary { operand } => {
                        self.collect_runtime_feature_uses_from_expression(
                            operand,
                            &statement.location,
                        );
                    }
                    HirNumericOperands::Binary { left, right } => {
                        self.collect_runtime_feature_uses_from_expression(
                            left,
                            &statement.location,
                        );
                        self.collect_runtime_feature_uses_from_expression(
                            right,
                            &statement.location,
                        );
                    }
                }
            }

            HirStatementKind::CastOp { source, .. } => {
                self.reachability
                    .reachable_runtime_casts
                    .push(ReachableRuntimeCastUse {
                        location: statement.location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(source, &statement.location);
            }

            HirStatementKind::FormatFloat { source, .. } => {
                self.reachability
                    .reachable_float_statements
                    .push(ReachableFloatStatementUse {
                        kind: ReachableFloatStatementKind::FormatFloat,
                        location: statement.location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(source, &statement.location);
            }

            HirStatementKind::ValidateFloat { source, .. } => {
                self.reachability
                    .reachable_float_statements
                    .push(ReachableFloatStatementUse {
                        kind: ReachableFloatStatementKind::ValidateFloat,
                        location: statement.location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(source, &statement.location);
            }
        }
    }

    fn collect_runtime_feature_uses_from_terminator(&mut self, block: &HirBlock) {
        let fallback_location = self
            .hir
            .side_table
            .hir_source_location_for_hir(HirLocation::Terminator(block.id))
            .cloned()
            .unwrap_or_default();

        match &block.terminator {
            // Terminators that carry a sub-expression to inspect.
            HirTerminator::If { condition, .. } => {
                self.collect_runtime_feature_uses_from_expression(condition, &fallback_location);
            }

            HirTerminator::FallibleBranch { result, .. } => {
                self.collect_runtime_feature_uses_from_expression(result, &fallback_location);
            }

            HirTerminator::Match { scrutinee, .. } => {
                self.collect_runtime_feature_uses_from_expression(scrutinee, &fallback_location);
            }

            // Terminators that return a value.
            HirTerminator::Return(value)
            | HirTerminator::ReturnSuccess(value)
            | HirTerminator::ReturnError(value) => {
                self.collect_runtime_feature_uses_from_expression(value, &fallback_location);
            }

            // Terminators with no sub-expressions to inspect.
            HirTerminator::Jump { .. }
            | HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::AssertFailure { .. }
            | HirTerminator::Uninitialized => {}
        }
    }

    fn collect_runtime_feature_uses_from_expression(
        &mut self,
        expression: &HirExpression,
        fallback_location: &SourceLocation,
    ) {
        let expression_location = self
            .hir
            .side_table
            .value_source_location(expression.id)
            .unwrap_or(fallback_location)
            .clone();

        // Only templates with actual runtime subscriptions are unsupported reactive runtime
        // features. Plain runtime templates with variable interpolations are snapshots, not live
        // reactive values, and are rejected by other backend-specific checks if needed.
        if let Some(template) = self
            .hir
            .side_table
            .reactive_template_for_value(expression.id)
            && !template.dependencies.is_empty()
        {
            self.reachability
                .reachable_reactive_templates
                .push(ReachableReactiveTemplateUse {
                    template_id: template.id,
                    location: expression_location.clone(),
                });
        }

        match &expression.kind {
            // Map literals.
            HirExpressionKind::MapLiteral(entries) => {
                self.reachability.reachable_map_uses.push(ReachableMapUse {
                    kind: ReachableMapUseKind::Literal,
                    location: expression_location.clone(),
                });
                for entry in entries {
                    self.collect_runtime_feature_uses_from_expression(
                        &entry.key,
                        &expression_location,
                    );
                    self.collect_runtime_feature_uses_from_expression(
                        &entry.value,
                        &expression_location,
                    );
                }
            }

            // Composite expressions: recurse into sub-expressions.
            HirExpressionKind::BinOp { left, right, .. } => {
                self.collect_runtime_feature_uses_from_expression(left, &expression_location);
                self.collect_runtime_feature_uses_from_expression(right, &expression_location);
            }

            HirExpressionKind::Cast {
                source: operand, ..
            } => {
                self.reachability
                    .reachable_runtime_casts
                    .push(ReachableRuntimeCastUse {
                        location: expression_location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(operand, &expression_location);
            }

            HirExpressionKind::UnaryOp { operand, .. }
            | HirExpressionKind::FallibleUnwrapSuccess { result: operand }
            | HirExpressionKind::FallibleUnwrapError { result: operand }
            | HirExpressionKind::VariantPayloadGet {
                source: operand, ..
            } => {
                self.collect_runtime_feature_uses_from_expression(operand, &expression_location);
            }

            HirExpressionKind::StructConstruct { fields, .. } => {
                for (_, value) in fields {
                    self.collect_runtime_feature_uses_from_expression(value, &expression_location);
                }
            }

            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => {
                for element in elements {
                    self.collect_runtime_feature_uses_from_expression(
                        element,
                        &expression_location,
                    );
                }
            }

            HirExpressionKind::Range { start, end } => {
                self.collect_runtime_feature_uses_from_expression(start, &expression_location);
                self.collect_runtime_feature_uses_from_expression(end, &expression_location);
            }

            HirExpressionKind::TupleGet { tuple, .. } => {
                self.collect_runtime_feature_uses_from_expression(tuple, &expression_location);
            }

            HirExpressionKind::VariantConstruct { fields, .. } => {
                for field in fields {
                    self.collect_runtime_feature_uses_from_expression(
                        &field.value,
                        &expression_location,
                    );
                }
            }

            // Leaf values: nothing to record.
            HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_)
            | HirExpressionKind::Load(_)
            | HirExpressionKind::Copy(_) => {}
        }
    }

    fn collect_reactive_sink_from_expression(
        &mut self,
        kind: ReachableReactiveSinkKind,
        expression: &HirExpression,
        fallback_location: &SourceLocation,
    ) {
        let Some(template) = self
            .hir
            .side_table
            .reactive_template_for_value(expression.id)
            .filter(|template| template.has_runtime_reactive_dependency())
        else {
            return;
        };

        let location = self
            .hir
            .side_table
            .value_source_location(expression.id)
            .unwrap_or(fallback_location)
            .clone();

        self.reachability
            .reachable_reactive_sinks
            .push(ReachableReactiveSinkUse {
                kind,
                template_id: template.id,
                location,
            });
    }

    fn enqueue_terminator_successors(
        &mut self,
        terminator: &HirTerminator,
    ) -> Result<(), CompilerError> {
        match terminator {
            HirTerminator::Jump { target, .. } => self.enqueue_block(*target),

            HirTerminator::If {
                then_block,
                else_block,
                ..
            } => {
                self.enqueue_block(*then_block);
                self.enqueue_block(*else_block);
            }

            HirTerminator::FallibleBranch {
                success_block,
                error_block,
                ..
            } => {
                self.enqueue_block(*success_block);
                self.enqueue_block(*error_block);
            }

            HirTerminator::Match { arms, .. } => {
                for arm in arms {
                    self.enqueue_block(arm.body);
                }
            }

            HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                self.enqueue_block(*target);
            }

            HirTerminator::Return(_)
            | HirTerminator::ReturnSuccess(_)
            | HirTerminator::ReturnError(_)
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::AssertFailure { .. } => {}

            HirTerminator::Uninitialized => {
                return Err(hir_reachability_error(
                    "Uninitialized HIR terminator reached HIR reachability analysis",
                ));
            }
        }

        Ok(())
    }

    fn enqueue_function(&mut self, function_id: FunctionId) {
        if !self.reachability.reachable_functions.contains(&function_id) {
            self.function_worklist.push_back(function_id);
        }
    }

    fn enqueue_block(&mut self, block_id: BlockId) {
        if !self.reachability.reachable_blocks.contains(&block_id) {
            self.block_worklist.push_back(block_id);
        }
    }
}

fn build_function_map(
    hir: &HirModule,
) -> Result<FxHashMap<FunctionId, &HirFunction>, CompilerError> {
    let mut function_by_id = FxHashMap::default();

    for function in &hir.functions {
        if function_by_id.insert(function.id, function).is_some() {
            return Err(hir_reachability_error(format!(
                "Duplicate HIR function id {:?} reached HIR reachability analysis",
                function.id
            )));
        }
    }

    Ok(function_by_id)
}

fn build_block_map(hir: &HirModule) -> Result<FxHashMap<BlockId, &HirBlock>, CompilerError> {
    let mut block_by_id = FxHashMap::default();

    for block in &hir.blocks {
        if block_by_id.insert(block.id, block).is_some() {
            return Err(hir_reachability_error(format!(
                "Duplicate HIR block id {:?} reached HIR reachability analysis",
                block.id
            )));
        }
    }

    Ok(block_by_id)
}

fn hir_reachability_error(message: impl Into<String>) -> CompilerError {
    CompilerError::new(
        message,
        SourceLocation::default(),
        ErrorType::HirTransformation,
    )
}

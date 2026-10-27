//! Static Bool `if` specialisation at the executable AST boundary.
//!
//! WHAT: resolves ordinary statement and value-producing `if` conditions through the module's
//! folded-value resolver, retains only the selected lexical scope and records provisional generic
//! request ranges owned by inactive bodies.
//! WHY: both authored bodies finish frontend validation before selection, while terminality,
//! generated materialisation and HIR consume active executable control flow only.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_rewrite::{
    promote_replacement, replacement_input, rewrite_expression_children,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CompletedOperation, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, GenericRequestRange, GenericRequestSlot,
    IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::statements::value_production::analyze_branch_exits;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueLexicalScope,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use rustc_hash::{FxHashMap, FxHashSet};

use super::normalize_ast::TemplateNormalizationError;

/// One transactional static-control-flow candidate derived from an authored AST.
pub(super) struct StaticIfCandidate {
    ast: Vec<AstNode>,
    specialization: StaticIfSpecialization,
}

impl StaticIfCandidate {
    pub(super) fn prepare(
        authored_ast: &[AstNode],
        const_values: &mut ConstValueStore,
        expression_builder: &mut AstExpressionBuilder,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
        string_table: &mut StringTable,
        type_environment: &TypeEnvironment,
        external_package_registry: Option<&ExternalPackageRegistry>,
    ) -> Result<Self, TemplateNormalizationError> {
        let mut ast = authored_ast.to_vec();
        let specialization = StaticIfSpecialization::run(
            &mut ast,
            const_values,
            expression_builder,
            template_ir_store,
            string_table,
            type_environment,
            external_package_registry,
        )?;

        Ok(Self {
            ast,
            specialization,
        })
    }

    pub(super) fn has_selections(&self) -> bool {
        self.specialization.has_selections()
    }

    pub(super) fn ast(&self) -> &[AstNode] {
        &self.ast
    }

    pub(super) fn ast_mut(&mut self) -> &mut [AstNode] {
        &mut self.ast
    }

    /// Publishes the candidate only when it contains selected static control flow.
    pub(super) fn publish(self, authored_ast: &mut Vec<AstNode>) -> StaticIfSpecialization {
        if self.specialization.has_selections() {
            *authored_ast = self.ast;
        }

        self.specialization
    }
}

/// Committed request lanes after static selection and executable-first deduplication.
pub(super) struct CommittedGenericRequests {
    pub(super) executable: Vec<GenericFunctionInstantiationRequest>,
    pub(super) assertion_validation: Vec<GenericFunctionInstantiationRequest>,
}

pub(super) struct StaticIfSpecialization {
    inactive_generic_requests: Vec<GenericRequestRange>,
    selection_count: usize,
    function_provenance: FxHashMap<PathId, SyntheticInterfaceProvenance>,
}

impl StaticIfSpecialization {
    pub(super) fn merge(&mut self, mut other: Self) {
        self.inactive_generic_requests
            .append(&mut other.inactive_generic_requests);
        self.selection_count += other.selection_count;
        for (function_path, provenance) in other.function_provenance.drain() {
            self.function_provenance
                .entry(function_path)
                .or_default()
                .merge(&provenance);
        }
    }

    pub(super) fn has_selections(&self) -> bool {
        self.selection_count != 0
    }

    pub(super) fn function_provenance(&self) -> &FxHashMap<PathId, SyntheticInterfaceProvenance> {
        &self.function_provenance
    }

    pub(super) fn run(
        ast_nodes: &mut [AstNode],
        const_values: &mut ConstValueStore,
        expression_builder: &mut AstExpressionBuilder,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
        string_table: &mut StringTable,
        type_environment: &TypeEnvironment,
        external_package_registry: Option<&ExternalPackageRegistry>,
    ) -> Result<Self, TemplateNormalizationError> {
        let module_environment = module_const_environment(const_values);
        let resolver = ConstValueResolver::new(
            string_table,
            const_values,
            expression_builder,
            template_ir_store,
            type_environment,
            external_package_registry,
        );
        let mut specializer = StaticIfSpecializer {
            resolver,
            inactive_generic_requests: Vec::new(),
            selection_count: 0,
            current_function_path: None,
            function_provenance: FxHashMap::default(),
            expression_pending: Vec::new(),
            expression_results: Vec::new(),
        };

        for node in ast_nodes {
            let mut environment = module_environment.clone();
            specializer.specialize_node(node, &mut environment)?;
        }

        Ok(Self {
            inactive_generic_requests: specializer.inactive_generic_requests,
            selection_count: specializer.selection_count,
            function_provenance: specializer.function_provenance,
        })
    }

    /// Commits executable demand and surviving inactive-assertion validation demand separately.
    ///
    /// Static selection precedes deduplication, and executable demand wins shared instances.
    /// Validation candidates must also belong to a retained concrete assertion witness, never
    /// an eliminated branch or an abstract generic validation body.
    pub(super) fn commit_generic_requests(
        self,
        mut slots: Vec<GenericRequestSlot>,
        assertion_validation_paths: &FxHashSet<PathId>,
    ) -> Result<CommittedGenericRequests, InfrastructureFailure> {
        for range in &self.inactive_generic_requests {
            let inactive = slots.get_mut(range.start..range.end).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "static-if inactive generic request range falls outside the request ledger",
                )
            })?;
            for slot in inactive {
                if matches!(
                    slot,
                    GenericRequestSlot::Pending | GenericRequestSlot::AssertionValidationPending
                ) {
                    return Err(pending_generic_request_error());
                }
                *slot = GenericRequestSlot::Dropped;
            }
        }

        let (kept, active_count, validation_count) = {
            let mut active_keys = FxHashSet::default();
            let mut kept = Vec::with_capacity(slots.len());
            let mut active_count = 0;
            for slot in &slots {
                let keep = match slot {
                    GenericRequestSlot::Pending
                    | GenericRequestSlot::AssertionValidationPending => {
                        return Err(pending_generic_request_error());
                    }
                    GenericRequestSlot::Dropped | GenericRequestSlot::AssertionValidation(_) => {
                        false
                    }
                    GenericRequestSlot::Concrete(request) => active_keys.insert(&request.key),
                };
                active_count += usize::from(keep);
                kept.push(keep);
            }
            let mut validation_keys = FxHashSet::default();
            let mut validation_count = 0;
            for (slot, kept) in slots.iter().zip(&mut kept) {
                if let GenericRequestSlot::AssertionValidation(request) = slot {
                    *kept = assertion_validation_paths.contains(&request.instance_path)
                        && !active_keys.contains(&request.key)
                        && validation_keys.insert(&request.key);
                    validation_count += usize::from(*kept);
                }
            }
            (kept, active_count, validation_count)
        };

        let mut active = Vec::with_capacity(active_count);
        let mut assertion_validation = Vec::with_capacity(validation_count);
        for (slot, kept) in slots.into_iter().zip(kept) {
            match slot {
                GenericRequestSlot::Concrete(request) if kept => active.push(request),
                GenericRequestSlot::AssertionValidation(request) if kept => {
                    assertion_validation.push(request);
                }
                _ => {}
            }
        }
        Ok(CommittedGenericRequests {
            executable: active,
            assertion_validation,
        })
    }
}

struct StaticIfSpecializer<'a> {
    resolver: ConstValueResolver<'a>,
    inactive_generic_requests: Vec<GenericRequestRange>,
    selection_count: usize,
    current_function_path: Option<PathId>,
    function_provenance: FxHashMap<PathId, SyntheticInterfaceProvenance>,
    expression_pending: Vec<SpecializationFrame>,
    expression_results: Vec<(ExprId, ExprId)>,
}

enum SpecializationFrame {
    Visit(ExprId),
    Finish {
        expression: ExprId,
        child_count: usize,
    },
}

impl StaticIfSpecializer<'_> {
    fn specialize_body(
        &mut self,
        body: &mut [AstNode],
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for node in body {
            self.specialize_node(node, environment)?;
        }
        Ok(())
    }

    fn specialize_node(
        &mut self,
        node: &mut AstNode,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        let selected_body = match &mut node.kind {
            NodeKind::VariableDeclaration(declaration) => {
                if let Some(initializer) = declaration.value.initializer_mut() {
                    self.specialize_expression(initializer, environment)?;
                }
                self.install_local_const(declaration, environment)?;
                None
            }
            NodeKind::If(condition, then_body, else_body, branch_metadata) => {
                self.specialize_expression(condition, environment)?;
                let selection = self.resolve_static_bool(condition, environment)?;

                let mut then_environment = environment.clone();
                let then_provenance = self
                    .specialize_body_with_isolated_provenance(then_body, &mut then_environment)?;
                let else_provenance = if let Some(else_body) = else_body {
                    let mut else_environment = environment.clone();
                    self.specialize_body_with_isolated_provenance(else_body, &mut else_environment)?
                } else {
                    FxHashMap::default()
                };

                if let Some((select_then, provenance)) = selection {
                    self.record_selected_provenance(&provenance);
                    self.record_inactive_range(branch_metadata.request_ranges, select_then);
                    self.merge_provenance_map(if select_then {
                        then_provenance
                    } else {
                        else_provenance
                    });
                    let selected_scope = if select_then {
                        Some(branch_metadata.then_scope)
                    } else {
                        branch_metadata.else_scope
                    };
                    let body = if select_then {
                        std::mem::take(then_body)
                    } else {
                        else_body.take().unwrap_or_default()
                    };
                    Some((body, selected_scope))
                } else {
                    self.merge_provenance_map(then_provenance);
                    self.merge_provenance_map(else_provenance);
                    None
                }
            }
            NodeKind::LexicalScope { body } => {
                let mut nested_environment = environment.clone();
                self.specialize_body(body, &mut nested_environment)?;
                None
            }
            NodeKind::Match {
                scrutinee,
                arms,
                default,
                ..
            } => {
                self.specialize_expression(scrutinee, environment)?;
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.specialize_expression(guard, environment)?;
                    }
                    let mut arm_environment = environment.clone();
                    self.specialize_body(&mut arm.body, &mut arm_environment)?;
                }
                if let Some(default) = default {
                    let mut default_environment = environment.clone();
                    self.specialize_body(default, &mut default_environment)?;
                }
                None
            }
            NodeKind::RangeLoop { range, body, .. } => {
                self.specialize_expression(&mut range.start, environment)?;
                self.specialize_expression(&mut range.end, environment)?;
                if let Some(step) = &mut range.step {
                    self.specialize_expression(step, environment)?;
                }
                let mut loop_environment = environment.clone();
                self.specialize_body(body, &mut loop_environment)?;
                None
            }
            NodeKind::CollectionLoop { iterable, body, .. } => {
                self.specialize_expression(iterable, environment)?;
                let mut loop_environment = environment.clone();
                self.specialize_body(body, &mut loop_environment)?;
                None
            }
            NodeKind::WhileLoop(condition, body) => {
                self.specialize_expression(condition, environment)?;
                let mut loop_environment = environment.clone();
                self.specialize_body(body, &mut loop_environment)?;
                None
            }
            NodeKind::Function(function_path, _, body) => {
                let previous_function_path = self.current_function_path.replace(*function_path);
                let mut function_environment = environment.clone();
                let result = self.specialize_body(body, &mut function_environment);
                self.current_function_path = previous_function_path;
                result?;
                None
            }
            NodeKind::Assert { condition, message } => {
                self.specialize_expression(condition, environment)?;
                self.specialize_expression(message, environment)?;
                None
            }
            NodeKind::Return(expressions) => {
                self.specialize_expressions(expressions, environment)?;
                None
            }
            NodeKind::ThenValue(values) => {
                self.specialize_expressions(&mut values.expressions, environment)?;
                None
            }
            NodeKind::ReturnError(expression)
            | NodeKind::PushStartRuntimeFragment(expression)
            | NodeKind::ExpressionStatement(expression) => {
                self.specialize_expression(expression, environment)?;
                None
            }
            NodeKind::Assignment { value, .. } | NodeKind::MultiBind { value, .. } => {
                self.specialize_expression(value, environment)?;
                None
            }
            NodeKind::StructDefinition(_, fields) => {
                for field in fields {
                    if let Some(initializer) = field.value.initializer_mut() {
                        self.specialize_expression(initializer, environment)?;
                    }
                }
                None
            }
            NodeKind::Break | NodeKind::Continue => None,
        };

        if let Some((body, selected_scope)) = selected_body {
            if let Some(selected_scope) = selected_scope {
                node.scope = selected_scope;
            }
            node.kind = NodeKind::LexicalScope { body };
        }

        if let Some((body, scope)) = take_terminal_receiver_body(
            &node.kind,
            self.resolver.builder(),
            self.resolver.const_values(),
        )? {
            node.scope = scope;
            node.kind = NodeKind::LexicalScope { body };
        }
        Ok(())
    }

    fn specialize_expressions(
        &mut self,
        expressions: &mut [ExprId],
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for expression in expressions {
            self.specialize_expression(expression, environment)?;
        }
        Ok(())
    }

    fn specialize_expression(
        &mut self,
        expression: &mut ExprId,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        // Value-body calls leave the enclosing walk's pending and result prefixes untouched.
        let pending_base = self.expression_pending.len();
        let root = self.resolver.builder().view(*expression)?;
        match root.operation() {
            ExpressionOperationView::Constant(_) | ExpressionOperationView::Reference(_) => {
                return Ok(());
            }
            ExpressionOperationView::Copy(_) => {
                root.visit_child_ids(&mut |id| {
                    self.expression_pending.push(SpecializationFrame::Visit(id));
                })?;
                let child_count = self.expression_pending.len() - pending_base;
                if child_count == 0 {
                    return Ok(());
                }
                self.expression_pending.push(SpecializationFrame::Finish {
                    expression: *expression,
                    child_count,
                });
                self.expression_pending[pending_base..].reverse();
            }
            _ => self
                .expression_pending
                .push(SpecializationFrame::Visit(*expression)),
        }
        while self.expression_pending.len() > pending_base {
            let frame = self
                .expression_pending
                .pop()
                .expect("active scratch suffix is non-empty");
            match frame {
                SpecializationFrame::Visit(expression_id) => {
                    let block = match self.resolver.builder().view(expression_id)?.operation() {
                        ExpressionOperationView::Constant(_)
                        | ExpressionOperationView::Reference(_) => {
                            self.expression_results.push((expression_id, expression_id));
                            continue;
                        }
                        ExpressionOperationView::ValueBlock(block) => Some(block.clone()),
                        _ => None,
                    };
                    if let Some(mut block) = block {
                        // Shared rows are immutable: selection transforms an independent body.
                        let previous_selection_count = self.selection_count;
                        self.specialize_value_block(&mut block, environment)?;
                        let replacement = if self.selection_count != previous_selection_count {
                            let original = self.resolver.builder().view(expression_id)?;
                            let input =
                                replacement_input(original, CompletedOperation::ValueBlock(block));
                            promote_replacement(self.resolver.builder_mut(), input)?.id()
                        } else {
                            expression_id
                        };
                        self.expression_results.push((expression_id, replacement));
                        continue;
                    }
                    let pending = &mut self.expression_pending;
                    let start = pending.len();
                    self.resolver
                        .builder()
                        .view(expression_id)?
                        .visit_child_ids(&mut |id| pending.push(SpecializationFrame::Visit(id)))?;
                    let child_count = pending.len() - start;
                    if child_count == 0 {
                        self.expression_results.push((expression_id, expression_id));
                        continue;
                    }
                    pending.push(SpecializationFrame::Finish {
                        expression: expression_id,
                        child_count,
                    });
                    pending[start..].reverse();
                }
                SpecializationFrame::Finish {
                    expression: original,
                    child_count,
                } => {
                    let start = self.expression_results.len() - child_count;
                    let original = self.resolver.builder().own(original);
                    let replacements = self
                        .resolver
                        .builder()
                        .own_slice(&self.expression_results[start..]);
                    let replacement = rewrite_expression_children(
                        self.resolver.builder_mut(),
                        original,
                        replacements,
                    )?;
                    self.expression_results.truncate(start);
                    self.expression_results
                        .push((original.id(), replacement.id()));
                }
            }
        }
        if let Some((_, replacement)) = self.expression_results.pop() {
            *expression = replacement;
        }
        Ok(())
    }

    fn specialize_value_block(
        &mut self,
        block: &mut ValueBlock,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        let selected = match block {
            ValueBlock::If(value_if) => {
                self.specialize_expression(&mut value_if.condition, environment)?;
                let selection = self.resolve_static_bool(&value_if.condition, environment)?;

                let mut then_environment = environment.clone();
                let then_provenance = self.specialize_body_with_isolated_provenance(
                    &mut value_if.then_body,
                    &mut then_environment,
                )?;
                let mut else_environment = environment.clone();
                let else_provenance = self.specialize_body_with_isolated_provenance(
                    &mut value_if.else_body,
                    &mut else_environment,
                )?;

                if let Some((select_then, provenance)) = selection {
                    self.record_selected_provenance(&provenance);
                    self.record_inactive_range(value_if.generic_request_ranges, select_then);
                    self.merge_provenance_map(if select_then {
                        then_provenance
                    } else {
                        else_provenance
                    });
                    let (body, scope) = if select_then {
                        (std::mem::take(&mut value_if.then_body), value_if.then_scope)
                    } else {
                        (std::mem::take(&mut value_if.else_body), value_if.else_scope)
                    };
                    Some(ValueLexicalScope {
                        body,
                        scope,
                        result_type_ids: value_if.result_type_ids.clone(),
                    })
                } else {
                    self.merge_provenance_map(then_provenance);
                    self.merge_provenance_map(else_provenance);
                    None
                }
            }
            ValueBlock::LexicalScope(value_lexical_scope) => {
                let mut lexical_environment = environment.clone();
                self.specialize_body(&mut value_lexical_scope.body, &mut lexical_environment)?;
                None
            }
            ValueBlock::Match(value_match) => {
                self.specialize_expression(&mut value_match.scrutinee, environment)?;
                for arm in &mut value_match.arms {
                    if let Some(guard) = &mut arm.guard {
                        self.specialize_expression(guard, environment)?;
                    }
                    let mut arm_environment = environment.clone();
                    self.specialize_body(&mut arm.body, &mut arm_environment)?;
                }
                if let Some(default) = &mut value_match.default {
                    let mut default_environment = environment.clone();
                    self.specialize_body(default, &mut default_environment)?;
                }
                None
            }
            ValueBlock::Catch(value_catch) => {
                self.specialize_expression(&mut value_catch.handled_value, environment)?;
                self.specialize_fallible_handling(&mut value_catch.handler, environment)?;
                None
            }
        };

        if let Some(selected) = selected {
            *block = ValueBlock::LexicalScope(selected);
        }
        Ok(())
    }

    fn specialize_fallible_handling(
        &mut self,
        handling: &mut FallibleHandling,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        if let FallibleHandling::Handler { body, .. } = handling {
            let mut handler_environment = environment.clone();
            self.specialize_body(body, &mut handler_environment)?;
        }
        Ok(())
    }

    fn resolve_static_bool(
        &mut self,
        condition: &ExprId,
        environment: &ConstValueEnvironment,
    ) -> Result<Option<(bool, SyntheticInterfaceProvenance)>, TemplateNormalizationError> {
        match self.resolver.resolve_expression(*condition, environment) {
            Ok(resolved) => match self.resolver.const_values().payload(resolved) {
                Some(ConstValuePayload::Bool(value)) => {
                    let view = self.resolver.builder().view(*condition)?;
                    let mut provenance = view
                        .occurrence_facts()
                        .map(|facts| facts.provenance.clone())
                        .unwrap_or_default();
                    if let Some(metadata) = self.resolver.const_values().metadata(resolved) {
                        provenance.merge(&metadata.synthetic_interface_provenance);
                    }
                    Ok(Some((*value, provenance)))
                }
                Some(_) | None => Ok(None),
            },
            Err(ConstResolutionError::TemplateClassification(error)) => {
                Err(TemplateNormalizationError::from(error))
            }
            Err(_) => Ok(None),
        }
    }

    fn specialize_body_with_isolated_provenance(
        &mut self,
        body: &mut [AstNode],
        environment: &mut ConstValueEnvironment,
    ) -> Result<FxHashMap<PathId, SyntheticInterfaceProvenance>, TemplateNormalizationError> {
        let saved_provenance = std::mem::take(&mut self.function_provenance);
        let result = self.specialize_body(body, environment);
        let branch_provenance = std::mem::take(&mut self.function_provenance);
        self.function_provenance = saved_provenance;
        result.map(|()| branch_provenance)
    }

    fn merge_provenance_map(
        &mut self,
        branch_provenance: FxHashMap<PathId, SyntheticInterfaceProvenance>,
    ) {
        for (function_path, provenance) in branch_provenance {
            if provenance.is_empty() {
                continue;
            }
            self.function_provenance
                .entry(function_path)
                .or_default()
                .merge(&provenance);
        }
    }

    fn record_selected_provenance(&mut self, provenance: &SyntheticInterfaceProvenance) {
        if provenance.is_empty() {
            return;
        }
        let Some(function_path) = &self.current_function_path else {
            return;
        };
        self.function_provenance
            .entry(*function_path)
            .or_default()
            .merge(provenance);
    }

    fn install_local_const(
        &mut self,
        declaration: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        if declaration.value_mode.is_mutable() {
            return Ok(());
        }
        let Some(initializer) = declaration.value.initializer() else {
            return Ok(());
        };
        match self.resolver.resolve_expression(initializer, environment) {
            Ok(expression) => environment.insert(declaration.id, expression),
            Err(ConstResolutionError::TemplateClassification(error)) => {
                return Err(TemplateNormalizationError::from(error));
            }
            Err(_) => {}
        }
        Ok(())
    }

    fn record_inactive_range(&mut self, ranges: IfGenericRequestRanges, select_then: bool) {
        self.selection_count += 1;
        self.inactive_generic_requests.push(if select_then {
            ranges.else_branch
        } else {
            ranges.then_branch
        });
    }
}

/// Error for a ledger slot that survived body completion without publishing its request.
fn pending_generic_request_error() -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        "generic request ledger kept a pending call after body completion",
    )
}

/// Removes a closed receiver when its selected value body terminates on every path.
///
/// The receiver never observes a value in this case. Keeping a value expression would force HIR
/// to invent an unreachable merge and result local after the selected return or error return.
fn take_terminal_receiver_body(
    kind: &NodeKind,
    builder: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> Result<Option<(Vec<AstNode>, PathId)>, InfrastructureFailure> {
    let expression = match kind {
        NodeKind::VariableDeclaration(declaration) => declaration.value.initializer(),
        NodeKind::Assignment { value, .. }
        | NodeKind::MultiBind { value, .. }
        | NodeKind::ReturnError(value) => Some(*value),
        NodeKind::Return(values) if values.len() == 1 => Some(values[0]),
        _ => None,
    };
    match expression {
        Some(expression) => take_terminal_value_body(expression, builder, constants),
        None => Ok(None),
    }
}

fn take_terminal_value_body(
    expression: ExprId,
    builder: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> Result<Option<(Vec<AstNode>, PathId)>, InfrastructureFailure> {
    match builder.view(expression)?.operation() {
        ExpressionOperationView::ValueBlock(ValueBlock::LexicalScope(scope)) => {
            let exits = analyze_branch_exits(&scope.body, builder, constants);
            if exits.terminates && !exits.produces_value && !exits.can_fall_through {
                // The immutable expression may still be read by another candidate.
                Ok(Some((scope.body.clone(), scope.scope)))
            } else {
                Ok(None)
            }
        }
        ExpressionOperationView::Coerce { value, .. } => {
            take_terminal_value_body(value.id(), builder, constants)
        }
        _ => Ok(None),
    }
}

fn module_const_environment(const_values: &ConstValueStore) -> ConstValueEnvironment {
    let module = const_values
        .iter_module_constant_views()
        .map(|row| (*row.path, row.id))
        .collect::<FxHashMap<_, _>>();
    ConstValueEnvironment::with_module_base(module)
}

#[cfg(test)]
#[path = "../tests/static_if_specialization_tests.rs"]
mod static_if_specialization_tests;

#[cfg(test)]
#[path = "tests/static_if_stack_tests.rs"]
mod stack_tests;

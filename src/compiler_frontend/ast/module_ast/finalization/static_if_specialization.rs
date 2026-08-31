//! Static Bool `if` specialisation at the executable AST boundary.
//!
//! WHAT: resolves ordinary statement and value-producing `if` conditions through the module's
//! folded-value resolver, retains only the selected scoped body and records provisional generic
//! request ranges owned by inactive bodies.
//! WHY: both authored bodies finish frontend validation before selection, while terminality,
//! generated materialisation and HIR consume active executable control flow only.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, GenericRequestRange, IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::statements::value_production::analyze_branch_exits;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueScopedBlock,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
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
        const_values: &ConstValueStore,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
        string_table: &mut StringTable,
    ) -> Result<Self, TemplateNormalizationError> {
        let mut ast = authored_ast.to_vec();
        let specialization =
            StaticIfSpecialization::run(&mut ast, const_values, template_ir_store, string_table)?;

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

pub(super) struct StaticIfSpecialization {
    inactive_generic_requests: Vec<GenericRequestRange>,
    selection_count: usize,
}

impl StaticIfSpecialization {
    pub(super) fn merge(&mut self, mut other: Self) {
        self.inactive_generic_requests
            .append(&mut other.inactive_generic_requests);
        self.selection_count += other.selection_count;
    }

    pub(super) fn has_selections(&self) -> bool {
        self.selection_count != 0
    }

    pub(super) fn run(
        ast_nodes: &mut [AstNode],
        const_values: &ConstValueStore,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
        string_table: &mut StringTable,
    ) -> Result<Self, TemplateNormalizationError> {
        let module_environment = module_const_environment(const_values);
        let resolver = ConstValueResolver::new(string_table, const_values, template_ir_store);
        let mut specializer = StaticIfSpecializer {
            resolver,
            inactive_generic_requests: Vec::new(),
            selection_count: 0,
        };

        for node in ast_nodes {
            let mut environment = module_environment.clone();
            specializer.specialize_node(node, &mut environment)?;
        }

        Ok(Self {
            inactive_generic_requests: specializer.inactive_generic_requests,
            selection_count: specializer.selection_count,
        })
    }

    pub(super) fn commit_active_generic_requests(
        self,
        requests: Vec<GenericFunctionInstantiationRequest>,
    ) -> Vec<GenericFunctionInstantiationRequest> {
        let mut seen = FxHashSet::default();
        requests
            .into_iter()
            .enumerate()
            .filter(|(index, _)| {
                !self
                    .inactive_generic_requests
                    .iter()
                    .any(|range| range.start <= *index && *index < range.end)
            })
            .filter_map(|(_, request)| seen.insert(request.key.clone()).then_some(request))
            .collect()
    }
}

struct StaticIfSpecializer<'a> {
    resolver: ConstValueResolver<'a>,
    inactive_generic_requests: Vec<GenericRequestRange>,
    selection_count: usize,
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
                self.specialize_expression(&mut declaration.value, environment)?;
                self.install_local_const(declaration, environment)?;
                None
            }
            NodeKind::If(condition, then_body, else_body, branch_metadata) => {
                self.specialize_expression(condition, environment)?;
                let selection = self.resolve_static_bool(condition, environment)?;

                let mut then_environment = environment.clone();
                self.specialize_body(then_body, &mut then_environment)?;
                if let Some(else_body) = else_body {
                    let mut else_environment = environment.clone();
                    self.specialize_body(else_body, &mut else_environment)?;
                }

                selection.map(|select_then| {
                    self.record_inactive_range(branch_metadata.request_ranges, select_then);
                    let selected_scope = if select_then {
                        Some(branch_metadata.then_scope.clone())
                    } else {
                        branch_metadata.else_scope.clone()
                    };
                    let body = if select_then {
                        std::mem::take(then_body)
                    } else {
                        else_body.take().unwrap_or_default()
                    };
                    (body, selected_scope)
                })
            }
            NodeKind::ScopedBlock { body } => {
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
            NodeKind::Function(_, _, body) => {
                let mut function_environment = environment.clone();
                self.specialize_body(body, &mut function_environment)?;
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
                    self.specialize_expression(&mut field.value, environment)?;
                }
                None
            }
            NodeKind::Break | NodeKind::Continue => None,
        };

        if let Some((body, selected_scope)) = selected_body {
            if let Some(selected_scope) = selected_scope {
                node.scope = selected_scope;
            }
            node.kind = NodeKind::ScopedBlock { body };
        }

        if let Some((body, scope)) = take_terminal_receiver_body(&mut node.kind) {
            node.scope = scope;
            node.kind = NodeKind::ScopedBlock { body };
        }
        Ok(())
    }

    fn specialize_expressions(
        &mut self,
        expressions: &mut [Expression],
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for expression in expressions {
            self.specialize_expression(expression, environment)?;
        }
        Ok(())
    }

    fn specialize_expression(
        &mut self,
        expression: &mut Expression,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match &mut expression.kind {
            ExpressionKind::Runtime(rpn) => {
                for item in &mut rpn.items {
                    if let ExpressionRpnItem::Operand(operand) = item {
                        self.specialize_expression(operand, environment)?;
                    }
                }
            }
            ExpressionKind::Copy(_) => {}
            ExpressionKind::FieldAccess { base, .. } => {
                self.specialize_expression(base, environment)?;
            }
            ExpressionKind::MethodCall { receiver, args, .. }
            | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
            | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
                self.specialize_expression(receiver, environment)?;
                for argument in args {
                    self.specialize_expression(&mut argument.value, environment)?;
                }
            }
            ExpressionKind::FunctionCall { args, .. }
            | ExpressionKind::HostFunctionCall { args, .. }
            | ExpressionKind::HandledFallibleFunctionCall { args, .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
                for argument in args {
                    self.specialize_expression(&mut argument.value, environment)?;
                }
            }
            ExpressionKind::HandledFallibleExpression { value, .. } => {
                self.specialize_expression(value, environment)?;
            }
            ExpressionKind::Cast(cast) => {
                self.specialize_expression(&mut cast.source, environment)?;
            }
            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { value, .. } => {
                self.specialize_expression(value, environment)?;
            }
            ExpressionKind::OptionPropagation { value } | ExpressionKind::Coerced { value, .. } => {
                self.specialize_expression(value, environment)?;
            }
            ExpressionKind::Collection(items) => {
                self.specialize_expressions(items, environment)?;
            }
            ExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    self.specialize_expression(&mut entry.key, environment)?;
                    self.specialize_expression(&mut entry.value, environment)?;
                }
            }
            ExpressionKind::StructDefinition(fields) | ExpressionKind::StructInstance(fields) => {
                for field in fields {
                    self.specialize_expression(&mut field.value, environment)?;
                }
            }
            ExpressionKind::ChoiceConstruct { fields, .. } => {
                for field in fields {
                    self.specialize_expression(&mut field.value, environment)?;
                }
            }
            ExpressionKind::Range(start, end) => {
                self.specialize_expression(start, environment)?;
                self.specialize_expression(end, environment)?;
            }
            ExpressionKind::ValueBlock { block } => {
                self.specialize_value_block(block, environment)?;
            }
            ExpressionKind::NoValue
            | ExpressionKind::OptionNone
            | ExpressionKind::Int(_)
            | ExpressionKind::Float(_)
            | ExpressionKind::StringSlice(_)
            | ExpressionKind::StructuralString { .. }
            | ExpressionKind::Bool(_)
            | ExpressionKind::Char(_)
            | ExpressionKind::Reference(_)
            | ExpressionKind::Function(_)
            | ExpressionKind::Template(_)
            | ExpressionKind::RuntimeTemplateHandoff(_)
            | ExpressionKind::RuntimeSlotApplicationHandoff(_) => {}
        }
        Ok(())
    }

    fn specialize_value_block(
        &mut self,
        block: &mut Box<ValueBlock>,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        let selected = match block.as_mut() {
            ValueBlock::If(value_if) => {
                self.specialize_expression(&mut value_if.condition, environment)?;
                let selection = self.resolve_static_bool(&value_if.condition, environment)?;

                let mut then_environment = environment.clone();
                self.specialize_body(&mut value_if.then_body, &mut then_environment)?;
                let mut else_environment = environment.clone();
                self.specialize_body(&mut value_if.else_body, &mut else_environment)?;

                selection.map(|select_then| {
                    self.record_inactive_range(value_if.generic_request_ranges, select_then);
                    let (body, scope) = if select_then {
                        (
                            std::mem::take(&mut value_if.then_body),
                            value_if.then_scope.clone(),
                        )
                    } else {
                        (
                            std::mem::take(&mut value_if.else_body),
                            value_if.else_scope.clone(),
                        )
                    };
                    ValueScopedBlock {
                        body,
                        scope,
                        result_type_ids: value_if.result_type_ids.clone(),
                    }
                })
            }
            ValueBlock::Scoped(value_scoped) => {
                let mut scoped_environment = environment.clone();
                self.specialize_body(&mut value_scoped.body, &mut scoped_environment)?;
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
            **block = ValueBlock::Scoped(selected);
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
        condition: &Expression,
        environment: &ConstValueEnvironment,
    ) -> Result<Option<bool>, TemplateNormalizationError> {
        match self.resolver.resolve_expression(condition, environment) {
            Ok(resolved) => match resolved.kind {
                ExpressionKind::Bool(value) => Ok(Some(value)),
                _ => Ok(None),
            },
            Err(ConstResolutionError::TemplateClassification(error)) => {
                Err(TemplateNormalizationError::from(error))
            }
            Err(_) => Ok(None),
        }
    }

    fn install_local_const(
        &mut self,
        declaration: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        environment: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        if declaration.value.value_mode.is_mutable() {
            return Ok(());
        }
        match self
            .resolver
            .resolve_expression(&declaration.value, environment)
        {
            Ok(expression) => environment.insert(declaration.id.clone(), expression),
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

/// Removes a closed receiver when its selected value body terminates on every path.
///
/// The receiver never observes a value in this case. Keeping a value expression would force HIR
/// to invent an unreachable merge and result local after the selected return or error return.
fn take_terminal_receiver_body(kind: &mut NodeKind) -> Option<(Vec<AstNode>, InternedPath)> {
    let expression = match kind {
        NodeKind::VariableDeclaration(declaration) => &mut declaration.value,
        NodeKind::Assignment { value, .. }
        | NodeKind::MultiBind { value, .. }
        | NodeKind::ReturnError(value) => value,
        NodeKind::Return(values) if values.len() == 1 => &mut values[0],
        _ => return None,
    };

    take_terminal_value_body(expression)
}

fn take_terminal_value_body(expression: &mut Expression) -> Option<(Vec<AstNode>, InternedPath)> {
    match &mut expression.kind {
        ExpressionKind::ValueBlock { block } => {
            let ValueBlock::Scoped(value_scoped) = block.as_mut() else {
                return None;
            };
            let exits = analyze_branch_exits(&value_scoped.body);
            if exits.terminates && !exits.produces_value && !exits.can_fall_through {
                Some((
                    std::mem::take(&mut value_scoped.body),
                    value_scoped.scope.clone(),
                ))
            } else {
                None
            }
        }
        ExpressionKind::Coerced { value, .. } => take_terminal_value_body(value),
        _ => None,
    }
}

fn module_const_environment(const_values: &ConstValueStore) -> ConstValueEnvironment {
    let module = const_values
        .iter_module_constant_views()
        .map(|row| (row.path.clone(), row.id))
        .collect::<FxHashMap<_, _>>();
    ConstValueEnvironment::with_module_base(module)
}

#[cfg(test)]
#[path = "../tests/static_if_specialization_tests.rs"]
mod static_if_specialization_tests;

//! Const fact collection during AST finalization.
//!
//! WHAT: walks the finalized AST and collects const facts for explicit module
//!       constants, private inferred top-level start-body declarations, and
//!       body-local declarations.
//! WHY: separates the detailed walking logic from the main finalizer
//!      orchestration to keep `finalizer.rs` readable.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::facts::{
    AstConstDeclarationFact, AstConstFacts, ConstBindingScope, ConstBindingSource,
};
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

use super::normalize_ast::TemplateNormalizationError;

/// Collects const facts from the finalized AST after normalization.
pub(super) struct ConstFactCollector<'a> {
    resolver: ConstValueResolver<'a>,
    facts: AstConstFacts,
    module_explicit_env: ConstValueEnvironment,
    expression_pending: Vec<ExprId>,
}

impl<'a> ConstFactCollector<'a> {
    /// Creates a collector backed by the module TIR store.
    ///
    /// WHAT: threads the shared module store from `AstPhaseContext` so template
    ///       const classification reads each exact effective TIR view.
    /// WHY: module-local roots and store-owned overlays are the authority for
    ///      classification rather than reconstructed template structure.
    pub(super) fn new(
        string_table: &'a mut StringTable,
        const_values: &'a mut ConstValueStore,
        expression_builder: &'a mut AstExpressionBuilder,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
        type_environment: &'a TypeEnvironment,
        external_package_registry: Option<&'a ExternalPackageRegistry>,
    ) -> Self {
        Self {
            resolver: ConstValueResolver::new(
                string_table,
                const_values,
                expression_builder,
                template_ir_store,
                type_environment,
                external_package_registry,
            ),
            facts: AstConstFacts::default(),
            module_explicit_env: ConstValueEnvironment::default(),
            expression_pending: Vec::new(),
        }
    }

    /// Copy one advisory environment for a nested lexical scope.
    ///
    /// WHAT: clones the lexical overlay and attributes the copy.
    /// WHY: every function body and every nested block, `if` arm and generated lexical scope takes a copy of
    /// what is visible so inner declarations cannot leak outward. The module base is shared behind
    /// an `Rc`, so only bindings the scopes introduced themselves are duplicated; the counters
    /// record how much that is.
    fn scope_environment(env: &ConstValueEnvironment) -> ConstValueEnvironment {
        increment_ast_counter(AstCounter::ConstFactEnvironmentClones);
        add_ast_counter(AstCounter::ConstFactEnvironmentEntriesCloned, env.len());
        env.clone()
    }

    /// Collect const facts from module constants and AST nodes.
    ///
    /// WHAT: resolves explicit module constants first (so private and body-local
    ///       facts can reference them), then walks the start function body for
    ///       private top-level facts, then walks all other function bodies for
    ///       body-local facts.
    pub(super) fn collect(
        mut self,
        ast_nodes: &[AstNode],
        start_function_path: Option<&PathId>,
    ) -> Result<AstConstFacts, TemplateNormalizationError> {
        self.collect_explicit_top_level_facts();
        self.collect_private_and_body_local_facts(ast_nodes, start_function_path)?;
        Ok(self.facts)
    }

    // ------------------------------
    //  Explicit top-level constants
    // ------------------------------

    /// Register explicit module constants as facts and as the environment's module base.
    fn collect_explicit_top_level_facts(&mut self) {
        let mut module_base = FxHashMap::default();

        for row in self.resolver.const_values().iter_module_constant_views() {
            let (path, value_id) = (row.path, row.id);

            // Lexical environments refer to the sole constant content owner.
            module_base.insert(*path, value_id);

            self.facts.declarations.insert(
                *path,
                AstConstDeclarationFact {
                    declaration_path: *path,
                    scope: ConstBindingScope::ExplicitTopLevel,
                    source: ConstBindingSource::ExplicitHash,
                    // Explicit module values stay owned by the store; the fact is
                    // metadata only and carries no authored initializer span.
                    span: None,
                },
            );
        }

        self.module_explicit_env = ConstValueEnvironment::with_module_base(module_base);
    }

    // ------------------------------------
    //  Private top-level and body-local
    // ------------------------------------

    /// Walk AST nodes to collect private top-level and body-local const facts.
    fn collect_private_and_body_local_facts(
        &mut self,
        ast_nodes: &[AstNode],
        start_function_path: Option<&PathId>,
    ) -> Result<(), TemplateNormalizationError> {
        for node in ast_nodes {
            if let NodeKind::Function(path, _, body) = &node.kind {
                if start_function_path == Some(path) {
                    let mut start_env = Self::scope_environment(&self.module_explicit_env);
                    self.walk_start_body(body, &mut start_env)?;
                } else {
                    let mut function_env = Self::scope_environment(&self.module_explicit_env);
                    self.walk_body_local(body, &mut function_env)?;
                }
            }
        }

        Ok(())
    }

    // ------------------------------
    //  Start body walker
    // ------------------------------

    /// Walk the start function body.
    ///
    /// WHAT: direct children that are variable declarations become
    ///       `PrivateTopLevel` facts. Nested scopes are walked for `BodyLocal`
    ///       facts. Declarations that do not resolve as const are skipped
    ///       silently.
    fn walk_start_body(
        &mut self,
        nodes: &[AstNode],
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for node in nodes {
            match &node.kind {
                NodeKind::VariableDeclaration(declaration) => {
                    if let Some(initializer) = declaration.value.initializer() {
                        self.walk_expression_for_body_local(&initializer, env)?;
                    }
                    self.try_add_private_top_level_fact(declaration, env)?;
                }

                _ => {
                    self.walk_node_for_body_local(node, env)?;
                }
            }
        }

        Ok(())
    }

    /// Attempt to resolve a start-body declaration as a private top-level const fact.
    ///
    /// WHAT: records occurrence metadata and retains the resolved constant ID in
    ///       the lexical environment without reconstructing an expression.
    fn try_add_private_top_level_fact(
        &mut self,
        declaration: &Declaration,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match self.resolver.resolve_declaration(declaration, env) {
            Ok(resolved) => {
                let span = declaration.value.initializer().and_then(|id| {
                    self.resolver
                        .builder()
                        .view(id)
                        .ok()
                        .and_then(|view| view.span())
                });
                env.insert(declaration.id, resolved.value);
                self.facts.declarations.insert(
                    declaration.id,
                    AstConstDeclarationFact {
                        declaration_path: declaration.id,
                        scope: ConstBindingScope::PrivateTopLevel,
                        source: ConstBindingSource::InferredImmutable,
                        span,
                    },
                );
            }

            Err(error) if error.is_expected_non_const_resolution() => {
                // Not a const fact — skip silently. Mutable declarations,
                // forward references, and runtime expressions are all
                // intentionally omitted.
            }

            Err(error) => {
                template_classification_error(error)?;
            }
        }

        Ok(())
    }

    // ------------------------------
    //  Body-local walker
    // ------------------------------

    /// Walk a function body for body-local const facts.
    ///
    /// WHAT: all variable declarations inside function bodies (and nested
    ///       scopes) are attempted as `BodyLocal` facts. Each nested scope
    ///       receives a cloned environment so declarations do not leak outward.
    fn walk_body_local(
        &mut self,
        nodes: &[AstNode],
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for node in nodes {
            self.walk_node_for_body_local(node, env)?;
        }

        Ok(())
    }

    /// Walk a single AST node for body-local const facts.
    ///
    /// WHAT: dispatches over all [`NodeKind`] variants, cloning the environment
    ///       for nested scopes and attempting to register const declarations.
    fn walk_node_for_body_local(
        &mut self,
        node: &AstNode,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match &node.kind {
            NodeKind::VariableDeclaration(declaration) => {
                if let Some(initializer) = declaration.value.initializer() {
                    self.walk_expression_for_body_local(&initializer, env)?;
                }
                self.try_add_body_local_fact(declaration, env)?;
            }

            NodeKind::LexicalScope { body } => {
                let mut nested_env = Self::scope_environment(env);
                self.walk_body_local(body, &mut nested_env)?;
            }

            NodeKind::If(condition, then_body, else_body, _) => {
                self.walk_expression_for_body_local(condition, env)?;

                let mut then_env = Self::scope_environment(env);
                self.walk_body_local(then_body, &mut then_env)?;

                if let Some(else_body) = else_body {
                    let mut else_env = Self::scope_environment(env);
                    self.walk_body_local(else_body, &mut else_env)?;
                }
            }

            NodeKind::Assert { condition, message } => {
                self.walk_expression_for_body_local(condition, env)?;
                self.walk_expression_for_body_local(message, env)?;
            }

            NodeKind::Match {
                scrutinee,
                arms,
                default,
                ..
            } => {
                self.walk_expression_for_body_local(scrutinee, env)?;

                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.walk_expression_for_body_local(guard, env)?;
                    }

                    let mut arm_env = Self::scope_environment(env);
                    self.walk_body_local(&arm.body, &mut arm_env)?;
                }

                if let Some(default_body) = default {
                    let mut default_env = Self::scope_environment(env);
                    self.walk_body_local(default_body, &mut default_env)?;
                }
            }

            NodeKind::RangeLoop { range, body, .. } => {
                self.walk_expression_for_body_local(&range.start, env)?;
                self.walk_expression_for_body_local(&range.end, env)?;
                if let Some(step) = &range.step {
                    self.walk_expression_for_body_local(step, env)?;
                }

                let mut loop_env = Self::scope_environment(env);
                self.walk_body_local(body, &mut loop_env)?;
            }

            NodeKind::CollectionLoop { iterable, body, .. } => {
                self.walk_expression_for_body_local(iterable, env)?;

                let mut loop_env = Self::scope_environment(env);
                self.walk_body_local(body, &mut loop_env)?;
            }

            NodeKind::WhileLoop(condition, body) => {
                self.walk_expression_for_body_local(condition, env)?;

                let mut loop_env = Self::scope_environment(env);
                self.walk_body_local(body, &mut loop_env)?;
            }

            NodeKind::Function(_, _, body) => {
                let mut nested_env = Self::scope_environment(env);
                self.walk_body_local(body, &mut nested_env)?;
            }

            NodeKind::Return(expressions) => {
                self.walk_expressions_for_body_local(expressions, env)?;
            }

            NodeKind::ThenValue(produced_values) => {
                self.walk_expressions_for_body_local(&produced_values.expressions, env)?;
            }

            NodeKind::ReturnError(expression) | NodeKind::PushStartRuntimeFragment(expression) => {
                self.walk_expression_for_body_local(expression, env)?;
            }

            NodeKind::Assignment { value, .. } => {
                self.walk_expression_for_body_local(value, env)?;
            }

            NodeKind::MultiBind { value, .. } | NodeKind::ExpressionStatement(value) => {
                self.walk_expression_for_body_local(value, env)?;
            }

            NodeKind::StructDefinition(_, fields) => {
                for field in fields {
                    if let Some(initializer) = field.value.initializer() {
                        self.walk_expression_for_body_local(&initializer, env)?;
                    }
                }
            }

            // All other node kinds do not contain declarations or nested
            // bodies that need walking for const facts.
            NodeKind::Break | NodeKind::Continue => {}
        }

        Ok(())
    }

    /// Attempt to resolve a body-local declaration as a const fact.
    ///
    /// WHAT: records occurrence metadata and retains the resolved constant ID in
    ///       the lexical environment without reconstructing an expression.
    fn try_add_body_local_fact(
        &mut self,
        declaration: &Declaration,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match self.resolver.resolve_declaration(declaration, env) {
            Ok(resolved) => {
                let span = declaration.value.initializer().and_then(|id| {
                    self.resolver
                        .builder()
                        .view(id)
                        .ok()
                        .and_then(|view| view.span())
                });
                env.insert(declaration.id, resolved.value);
                self.facts.declarations.insert(
                    declaration.id,
                    AstConstDeclarationFact {
                        declaration_path: declaration.id,
                        scope: ConstBindingScope::BodyLocal,
                        source: ConstBindingSource::InferredImmutable,
                        span,
                    },
                );
            }

            Err(error) if error.is_expected_non_const_resolution() => {
                // Not a const fact — skip silently.
            }

            Err(error) => {
                template_classification_error(error)?;
            }
        }

        Ok(())
    }

    /// Walk a list of expressions for body-local const facts.
    fn walk_expressions_for_body_local(
        &mut self,
        expressions: &[ExprId],
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for expression in expressions {
            self.walk_expression_for_body_local(expression, env)?;
        }

        Ok(())
    }

    /// Visits ID-backed operations, isolating environments at value-body scopes.
    fn walk_expression_for_body_local(
        &mut self,
        expression: &ExprId,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        // Nested lexical walks consume their own suffix without disturbing the parent work.
        let pending_base = self.expression_pending.len();
        let root = self.resolver.builder().view(*expression)?;
        match root.operation() {
            ExpressionOperationView::Constant(_) | ExpressionOperationView::Reference(_) => {
                return Ok(());
            }
            ExpressionOperationView::Copy(_) => {
                root.visit_child_ids(&mut |id| self.expression_pending.push(id))?;
                if self.expression_pending.len() == pending_base {
                    return Ok(());
                }
                self.expression_pending[pending_base..].reverse();
            }
            _ => self.expression_pending.push(*expression),
        }
        while self.expression_pending.len() > pending_base {
            let expression = self
                .expression_pending
                .pop()
                .expect("active scratch suffix is non-empty");
            let view = self.resolver.builder().view(expression)?;
            let block = match view.operation() {
                ExpressionOperationView::Constant(_) | ExpressionOperationView::Reference(_) => {
                    continue;
                }
                ExpressionOperationView::ValueBlock(block) => Some(block.clone()),
                _ => None,
            };
            if block.is_none() {
                let pending = &mut self.expression_pending;
                let start = pending.len();
                view.visit_child_ids(&mut |id| pending.push(id))?;
                pending[start..].reverse();
                continue;
            }
            if let Some(block) = block {
                match &block {
                    ValueBlock::If(value_if) => {
                        self.walk_expression_for_body_local(&value_if.condition, env)?;
                        let mut then_env = Self::scope_environment(env);
                        self.walk_body_local(&value_if.then_body, &mut then_env)?;
                        let mut else_env = Self::scope_environment(env);
                        self.walk_body_local(&value_if.else_body, &mut else_env)?;
                    }
                    ValueBlock::LexicalScope(scope) => {
                        let mut nested_env = Self::scope_environment(env);
                        self.walk_body_local(&scope.body, &mut nested_env)?;
                    }
                    ValueBlock::Match(value_match) => {
                        self.walk_expression_for_body_local(&value_match.scrutinee, env)?;
                        for arm in &value_match.arms {
                            if let Some(guard) = &arm.guard {
                                self.walk_expression_for_body_local(guard, env)?;
                            }
                            let mut arm_env = Self::scope_environment(env);
                            self.walk_body_local(&arm.body, &mut arm_env)?;
                        }
                        if let Some(default) = &value_match.default {
                            let mut default_env = Self::scope_environment(env);
                            self.walk_body_local(default, &mut default_env)?;
                        }
                    }
                    ValueBlock::Catch(value_catch) => {
                        self.walk_expression_for_body_local(&value_catch.handled_value, env)?;
                        self.walk_fallible_handling_for_body_local(&value_catch.handler, env)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Walk fallible handler bodies for body-local const facts.
    ///
    /// WHAT: walks the handler body in an isolated environment.
    fn walk_fallible_handling_for_body_local(
        &mut self,
        handling: &FallibleHandling,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        let FallibleHandling::Handler { body, .. } = handling else {
            return Ok(());
        };

        let mut handler_env = Self::scope_environment(env);
        self.walk_body_local(body, &mut handler_env)
    }
}

fn template_classification_error(
    error: ConstResolutionError,
) -> Result<(), TemplateNormalizationError> {
    match error {
        ConstResolutionError::TemplateClassification(error) => {
            Err(TemplateNormalizationError::from(error))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "tests/const_fact_stack_tests.rs"]
mod stack_tests;

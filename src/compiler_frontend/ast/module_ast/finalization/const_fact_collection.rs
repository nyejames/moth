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
    AstConstDeclarationFact, AstConstFactValue, AstConstFacts, ConstBindingScope,
    ConstBindingSource, ConstFactValueKind,
};
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

use super::normalize_ast::TemplateNormalizationError;

/// Collects const facts from the finalized AST after normalization.
pub(super) struct ConstFactCollector<'a> {
    resolver: ConstValueResolver<'a>,
    facts: AstConstFacts,
    module_explicit_env: ConstValueEnvironment,
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
        const_values: &'a ConstValueStore,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
    ) -> Self {
        Self {
            resolver: ConstValueResolver::new(string_table, const_values, template_ir_store),
            facts: AstConstFacts::default(),
            module_explicit_env: ConstValueEnvironment::default(),
        }
    }

    /// Copy one advisory environment for a nested lexical scope.
    ///
    /// WHAT: clones the lexical overlay and attributes the copy.
    /// WHY: every function body and every nested block, `if` arm and scoped block takes a copy of
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
        const_values: &ConstValueStore,
        ast_nodes: &[AstNode],
        start_function_path: Option<&InternedPath>,
    ) -> Result<AstConstFacts, TemplateNormalizationError> {
        self.collect_explicit_top_level_facts(const_values);
        self.collect_private_and_body_local_facts(ast_nodes, start_function_path)?;
        Ok(self.facts)
    }

    // ------------------------------
    //  Explicit top-level constants
    // ------------------------------

    /// Register explicit module constants as facts and as the environment's module base.
    fn collect_explicit_top_level_facts(&mut self, const_values: &ConstValueStore) {
        let mut module_base = FxHashMap::default();

        for row in const_values.iter_module_constant_views() {
            let (path, value_id, metadata) = (row.path, row.id, row.metadata);

            // The base records the store id, not an expression. Body-local advisory resolution
            // still consumes `Expression` operands, so one is built at the reference that needs
            // it rather than for all of a module's constants up front.
            module_base.insert(path.clone(), value_id);

            self.facts.declarations.insert(
                path.clone(),
                AstConstDeclarationFact {
                    declaration_path: path.clone(),
                    scope: ConstBindingScope::ExplicitTopLevel,
                    source: ConstBindingSource::ExplicitHash,
                    value_kind: ConstFactValueKind::from_const_value_kind(metadata.value_kind),
                    value: AstConstFactValue::Stored(value_id),
                    location: metadata.location.clone(),
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
        start_function_path: Option<&InternedPath>,
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
                    self.walk_expression_for_body_local(&declaration.value, env)?;
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
    /// WHAT: on success, inserts the fact into both the local environment
    ///       and the output fact table.
    fn try_add_private_top_level_fact(
        &mut self,
        declaration: &Declaration,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match self
            .resolver
            .resolve_private_top_level_declaration(declaration, env)
        {
            Ok(fact) => {
                if let AstConstFactValue::Expression(expression) = &fact.value {
                    env.insert(declaration.id.clone(), expression.as_ref().clone());
                }
                self.facts.declarations.insert(declaration.id.clone(), fact);
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
                self.walk_expression_for_body_local(&declaration.value, env)?;
                self.try_add_body_local_fact(declaration, env)?;
            }

            NodeKind::ScopedBlock { body } => {
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
                    self.walk_match_pattern_for_body_local(&arm.pattern, env)?;
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
                    self.walk_expression_for_body_local(&field.value, env)?;
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
    /// WHAT: on success, inserts the fact into both the local environment
    ///       and the output fact table.
    fn try_add_body_local_fact(
        &mut self,
        declaration: &Declaration,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match self
            .resolver
            .resolve_body_local_declaration(declaration, env)
        {
            Ok(fact) => {
                if let AstConstFactValue::Expression(expression) = &fact.value {
                    env.insert(declaration.id.clone(), expression.as_ref().clone());
                }
                self.facts.declarations.insert(declaration.id.clone(), fact);
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

    /// Walk call arguments for body-local const facts.
    fn walk_call_arguments_for_body_local(
        &mut self,
        arguments: &[CallArgument],
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for argument in arguments {
            self.walk_expression_for_body_local(&argument.value, env)?;
        }

        Ok(())
    }

    /// Walk a list of expressions for body-local const facts.
    fn walk_expressions_for_body_local(
        &mut self,
        expressions: &[Expression],
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        for expression in expressions {
            self.walk_expression_for_body_local(expression, env)?;
        }

        Ok(())
    }

    /// Walk a match pattern for body-local const facts.
    ///
    /// WHAT: only literal, option-value, and relational patterns contain
    ///       nested expressions that need walking.
    fn walk_match_pattern_for_body_local(
        &mut self,
        pattern: &MatchPattern,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match pattern {
            MatchPattern::Literal(expression) => {
                self.walk_expression_for_body_local(expression, env)?;
            }

            MatchPattern::OptionValue { value, .. } | MatchPattern::Relational { value, .. } => {
                self.walk_expression_for_body_local(value, env)?;
            }

            MatchPattern::OptionNone { .. }
            | MatchPattern::ChoiceVariant { .. }
            | MatchPattern::OptionPresentCapture { .. } => {}
        }

        Ok(())
    }

    /// Walk an expression tree for body-local const facts.
    ///
    /// WHAT: recursively descends through nested AST nodes, function literals,
    ///       and fallible handling structures.
    /// WHY: expressions may contain scoped blocks or call arguments that
    ///      reference or declare const-foldable values.
    fn walk_place_expression_for_body_local(place: &PlaceExpression) {
        match &place.kind {
            PlaceExpressionKind::Local(_) => {}
            PlaceExpressionKind::Field { base, .. } => {
                Self::walk_place_expression_for_body_local(base)
            }
        }
    }

    fn walk_expression_for_body_local(
        &mut self,
        expression: &Expression,
        env: &mut ConstValueEnvironment,
    ) -> Result<(), TemplateNormalizationError> {
        match &expression.kind {
            ExpressionKind::Runtime(rpn) => {
                for item in &rpn.items {
                    match item {
                        ExpressionRpnItem::Operand(expression) => {
                            self.walk_expression_for_body_local(expression, env)?;
                        }
                        ExpressionRpnItem::Operator { .. } => {}
                    }
                }
            }

            ExpressionKind::Copy(place) => {
                Self::walk_place_expression_for_body_local(place);
            }

            ExpressionKind::FieldAccess { base, .. } => {
                self.walk_expression_for_body_local(base, env)?;
            }

            ExpressionKind::MethodCall { receiver, args, .. }
            | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
            | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
                self.walk_expression_for_body_local(receiver, env)?;
                self.walk_call_arguments_for_body_local(args, env)?;
            }

            ExpressionKind::FunctionCall { args, .. }
            | ExpressionKind::HostFunctionCall { args, .. } => {
                self.walk_call_arguments_for_body_local(args, env)?;
            }

            ExpressionKind::HandledFallibleFunctionCall { args, .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
                self.walk_call_arguments_for_body_local(args, env)?;
            }

            ExpressionKind::HandledFallibleExpression { value, .. } => {
                self.walk_expression_for_body_local(value, env)?;
            }

            ExpressionKind::Cast(cast) => {
                self.walk_expression_for_body_local(&cast.source, env)?;
            }

            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { value, .. } => {
                self.walk_expression_for_body_local(value, env)?;
            }

            ExpressionKind::OptionPropagation { value } | ExpressionKind::Coerced { value, .. } => {
                self.walk_expression_for_body_local(value, env)?;
            }

            ExpressionKind::Collection(items) => {
                self.walk_expressions_for_body_local(items, env)?;
            }

            ExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    self.walk_expression_for_body_local(&entry.key, env)?;
                    self.walk_expression_for_body_local(&entry.value, env)?;
                }
            }

            ExpressionKind::StructDefinition(fields) | ExpressionKind::StructInstance(fields) => {
                for field in fields {
                    self.walk_expression_for_body_local(&field.value, env)?;
                }
            }

            ExpressionKind::ChoiceConstruct { fields, .. } => {
                for field in fields {
                    self.walk_expression_for_body_local(&field.value, env)?;
                }
            }

            ExpressionKind::Range(start, end) => {
                self.walk_expression_for_body_local(start, env)?;
                self.walk_expression_for_body_local(end, env)?;
            }

            ExpressionKind::ValueBlock { block } => match block.as_ref() {
                ValueBlock::If(value_if) => {
                    self.walk_expression_for_body_local(&value_if.condition, env)?;

                    let mut then_env = Self::scope_environment(env);
                    self.walk_body_local(&value_if.then_body, &mut then_env)?;

                    let mut else_env = Self::scope_environment(env);
                    self.walk_body_local(&value_if.else_body, &mut else_env)?;
                }
                ValueBlock::Scoped(value_scoped) => {
                    let mut nested_env = Self::scope_environment(env);
                    self.walk_body_local(&value_scoped.body, &mut nested_env)?;
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

                    if let Some(default_body) = &value_match.default {
                        let mut default_env = Self::scope_environment(env);
                        self.walk_body_local(default_body, &mut default_env)?;
                    }
                }
                ValueBlock::Catch(value_catch) => {
                    self.walk_expression_for_body_local(&value_catch.handled_value, env)?;
                    self.walk_fallible_handling_for_body_local(&value_catch.handler, env)?;
                }
            },

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

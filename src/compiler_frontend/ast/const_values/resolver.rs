//! Const value resolution logic.
//!
//! WHAT: evaluates whether an AST expression resolves to a compile-time constant
//!       by substituting known const references and reusing the existing constant folder.
//! WHY: one shared resolver avoids duplicating fold/reference logic across config,
//!      AST finalization, and HIR metadata.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_eval::constant_fold;
use crate::compiler_frontend::ast::const_values::facts::{
    AstConstDeclarationFact, ConstBindingScope, ConstBindingSource, ConstFactValueKind,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
#[cfg(test)]
use crate::compiler_frontend::ast::templates::tir::TemplatePreparationFacts;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplatePreparationMode, TemplateTirPhase, TirView, prepare_tir_view,
};
#[cfg(test)]
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

/// Lookup table for resolved const bindings visible in the current scope.
///
/// WHAT: maps interned declaration path to the fully resolved const expression.
/// WHY: reference resolution needs a narrow, explicit environment instead of
///      reaching into broader AST or scope context structures.
#[derive(Clone, Debug, Default)]
pub struct ConstValueEnvironment {
    bindings: FxHashMap<InternedPath, Expression>,
}

impl ConstValueEnvironment {
    /// Insert a resolved const binding into the environment.
    pub fn insert(&mut self, path: InternedPath, expression: Expression) {
        self.bindings.insert(path, expression);
    }

    /// Look up a const binding by path.
    pub fn lookup(&self, path: &InternedPath) -> Option<&Expression> {
        self.bindings.get(path)
    }
}

/// Reason why an expression could not be resolved to a compile-time constant.
///
/// WHAT: structured failure cases for const resolution.
/// WHY: callers decide how to report or ignore failures; the resolver does not
///      emit user-facing diagnostics directly.
#[derive(Debug)]
pub enum ConstResolutionError {
    UnresolvedReference,
    NonConstReference,
    NonFoldableRuntimeExpression,
    CallInConstContext,
    MutableDeclaration,
    NonConstExpression,
    TemplateClassification(TemplateError),
}

impl ConstResolutionError {
    /// Expected non-const failures are advisory for fact collection.
    ///
    /// WHAT: unresolved references, mutable declarations, calls, and runtime
    ///       expressions simply mean "do not record a const fact". Template
    ///       classification errors are different because they may represent a
    ///       broken TIR materialization invariant or a source diagnostic that
    ///       should stay on the template normalization boundary.
    pub(crate) fn is_expected_non_const_resolution(&self) -> bool {
        !matches!(self, Self::TemplateClassification(_))
    }
}

impl From<TemplateError> for ConstResolutionError {
    fn from(error: TemplateError) -> Self {
        Self::TemplateClassification(error)
    }
}

impl PartialEq for ConstResolutionError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::UnresolvedReference, Self::UnresolvedReference)
                | (Self::NonConstReference, Self::NonConstReference)
                | (
                    Self::NonFoldableRuntimeExpression,
                    Self::NonFoldableRuntimeExpression
                )
                | (Self::CallInConstContext, Self::CallInConstContext)
                | (Self::MutableDeclaration, Self::MutableDeclaration)
                | (Self::NonConstExpression, Self::NonConstExpression)
                | (
                    Self::TemplateClassification(_),
                    Self::TemplateClassification(_)
                )
        )
    }
}

impl Eq for ConstResolutionError {}

/// Resolves AST expressions against a [`ConstValueEnvironment`] to determine
/// whether they are compile-time constants.
pub struct ConstValueResolver<'a> {
    string_table: &'a mut StringTable,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
}

impl<'a> ConstValueResolver<'a> {
    /// Creates a resolver backed by the module TIR store.
    ///
    /// WHAT: the caller supplies the module store so each template is
    ///       classified through its exact effective TIR view.
    /// WHY: const-fact collection runs after template normalization and must
    ///      classify each template through its exact module-local view, including
    ///      its overlay identity.
    pub fn new(
        string_table: &'a mut StringTable,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
    ) -> Self {
        Self {
            string_table,
            template_ir_store,
        }
    }

    // ------------------------------
    //  Declaration resolution
    // ------------------------------

    /// Resolve an explicit `#=` top-level constant declaration.
    ///
    /// WHAT: explicit constants are const by syntax; this resolves their initializer
    ///       expression through the environment and builds a fact.
    pub fn resolve_explicit_top_level_constant(
        &mut self,
        declaration: &Declaration,
        environment: &ConstValueEnvironment,
    ) -> Result<AstConstDeclarationFact, ConstResolutionError> {
        let resolved = self.resolve_expression(&declaration.value, environment)?;
        let value_kind = self.fact_value_kind(&resolved)?;

        Ok(AstConstDeclarationFact {
            declaration_path: declaration.id.clone(),
            scope: ConstBindingScope::ExplicitTopLevel,
            source: ConstBindingSource::ExplicitHash,
            value_kind,
            resolved_expression: resolved,
            location: declaration.value.location.clone(),
        })
    }

    /// Resolve a private inferred top-level declaration (`=` in start body).
    ///
    /// WHAT: mutable declarations are rejected; immutable declarations are const
    ///       only when their initializer fully resolves.
    pub fn resolve_private_top_level_declaration(
        &mut self,
        declaration: &Declaration,
        environment: &ConstValueEnvironment,
    ) -> Result<AstConstDeclarationFact, ConstResolutionError> {
        if declaration.value.value_mode.is_mutable() {
            return Err(ConstResolutionError::MutableDeclaration);
        }

        let resolved = self.resolve_expression(&declaration.value, environment)?;
        let value_kind = self.fact_value_kind(&resolved)?;

        Ok(AstConstDeclarationFact {
            declaration_path: declaration.id.clone(),
            scope: ConstBindingScope::PrivateTopLevel,
            source: ConstBindingSource::InferredImmutable,
            value_kind,
            resolved_expression: resolved,
            location: declaration.value.location.clone(),
        })
    }

    /// Resolve a body-local private inferred declaration.
    ///
    /// WHAT: same rules as [`Self::resolve_private_top_level_declaration`] but
    ///       tagged with [`ConstBindingScope::BodyLocal`].
    pub fn resolve_body_local_declaration(
        &mut self,
        declaration: &Declaration,
        environment: &ConstValueEnvironment,
    ) -> Result<AstConstDeclarationFact, ConstResolutionError> {
        if declaration.value.value_mode.is_mutable() {
            return Err(ConstResolutionError::MutableDeclaration);
        }

        let resolved = self.resolve_expression(&declaration.value, environment)?;
        let value_kind = self.fact_value_kind(&resolved)?;

        Ok(AstConstDeclarationFact {
            declaration_path: declaration.id.clone(),
            scope: ConstBindingScope::BodyLocal,
            source: ConstBindingSource::InferredImmutable,
            value_kind,
            resolved_expression: resolved,
            location: declaration.value.location.clone(),
        })
    }

    // ------------------------------
    //  Expression resolution
    // ------------------------------

    /// Resolve an arbitrary expression against the given environment.
    ///
    /// WHAT: the core resolution algorithm that handles literals, references,
    ///       runtime RPN, and coercion nodes.
    pub fn resolve_expression(
        &mut self,
        expression: &Expression,
        environment: &ConstValueEnvironment,
    ) -> Result<Expression, ConstResolutionError> {
        // Fast path: expressions that are already compile-time constants
        // (literals, composite collections, templates, etc.) need no substitution.
        if self.is_compile_time_constant(expression)? {
            return Ok(expression.clone());
        }

        match &expression.kind {
            ExpressionKind::Reference(path) => self.resolve_reference(path, environment),

            ExpressionKind::Runtime(rpn) => self.resolve_runtime_rpn(rpn, environment),

            ExpressionKind::Coerced { value, .. } => {
                // A coercion does not change whether the inner value is const.
                self.resolve_expression(value, environment)
            }

            // Any call shape is treated as non-const. This includes function calls,
            // host calls, handled fallible calls, collection builtins, and method
            // calls (the latter two appear inside Runtime RPN, not as ExpressionKind).
            ExpressionKind::FunctionCall { .. }
            | ExpressionKind::HostFunctionCall { .. }
            | ExpressionKind::HandledFallibleFunctionCall { .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { .. }
            | ExpressionKind::MethodCall { .. }
            | ExpressionKind::CollectionBuiltinCall { .. }
            | ExpressionKind::MapBuiltinCall { .. }
            | ExpressionKind::FieldAccess { .. } => Err(ConstResolutionError::CallInConstContext),

            _ => Err(ConstResolutionError::NonConstExpression),
        }
    }

    // ------------------------------
    //  Internal helpers
    // ------------------------------

    fn resolve_reference(
        &mut self,
        path: &InternedPath,
        environment: &ConstValueEnvironment,
    ) -> Result<Expression, ConstResolutionError> {
        let resolved = environment
            .lookup(path)
            .ok_or(ConstResolutionError::UnresolvedReference)?;

        if self.is_compile_time_constant(resolved)? {
            Ok(resolved.clone())
        } else {
            Err(ConstResolutionError::NonConstReference)
        }
    }

    /// Substitute known const references into an RPN stack, fold, and accept
    /// only when the result is a single compile-time expression.
    fn resolve_runtime_rpn(
        &mut self,
        rpn: &ExpressionRpn,
        environment: &ConstValueEnvironment,
    ) -> Result<Expression, ConstResolutionError> {
        let mut substituted = Vec::with_capacity(rpn.items.len());

        for item in &rpn.items {
            let new_item = match item {
                ExpressionRpnItem::Operand(expression) => {
                    self.resolve_runtime_rvalue_operand(expression, environment)?
                }
                operator @ ExpressionRpnItem::Operator { .. } => operator.clone(),
            };
            substituted.push(new_item);
        }

        let stack = constant_fold(&substituted, self.string_table)
            .map_err(|_| ConstResolutionError::NonFoldableRuntimeExpression)?;

        if stack.len() == 1
            && let ExpressionRpnItem::Operand(expression) = &stack[0]
            && self.is_compile_time_constant(expression)?
        {
            return Ok(expression.clone());
        }

        Err(ConstResolutionError::NonFoldableRuntimeExpression)
    }

    fn resolve_runtime_rvalue_operand(
        &mut self,
        expression: &Expression,
        environment: &ConstValueEnvironment,
    ) -> Result<ExpressionRpnItem, ConstResolutionError> {
        let resolved = match &expression.kind {
            ExpressionKind::Reference(..) | ExpressionKind::Coerced { .. } => {
                Some(self.resolve_expression(expression, environment)?)
            }
            _ => None,
        };

        if let Some(resolved_expression) = resolved {
            return Ok(ExpressionRpnItem::Operand(resolved_expression));
        }

        Ok(ExpressionRpnItem::Operand(expression.clone()))
    }

    fn fact_value_kind(
        &mut self,
        expression: &Expression,
    ) -> Result<ConstFactValueKind, ConstResolutionError> {
        let kind = self.const_value_kind(expression)?;
        Ok(ConstFactValueKind::from_const_value_kind(kind))
    }

    fn is_compile_time_constant(
        &mut self,
        expression: &Expression,
    ) -> Result<bool, ConstResolutionError> {
        Ok(self.const_value_kind(expression)?.is_compile_time_value())
    }

    fn const_value_kind(
        &mut self,
        expression: &Expression,
    ) -> Result<ConstValueKind, ConstResolutionError> {
        let store = Rc::clone(&self.template_ir_store);

        expression
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, &store)
            })
            .map_err(ConstResolutionError::from)
    }
}

/// Classifies one template through its module-local effective TIR view.
///
/// WHAT: validates the module-local reference, phase and overlay identity before
///       using preparation facts from the effective view on the shared module store.
/// WHY: AST const consumers run after composition, so missing or pre-Composed
///      identity is a broken phase invariant rather than permission to recover
///      semantics outside the exact module-local view.
pub(crate) fn classify_template_from_effective_tir(
    template: &Template,
    store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<TemplateConstValueKind, TemplateError> {
    let reference = &template.tir_reference;
    let store = store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)?;
    Ok(preparation.facts.final_value_kind)
}

#[cfg(test)]
pub(crate) fn prepare_template_tir_facts(
    template: &Template,
    store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<TemplatePreparationFacts, TemplateError> {
    let reference = &template.tir_reference;

    if !reference.phase.is_at_least(TemplateTirPhase::Composed) {
        return Err(CompilerError::compiler_error(format!(
            "AST const template preparation requires Composed TIR, but root {} is at phase {}.",
            reference.root, reference.phase
        ))
        .into());
    }

    let store = store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    Ok(prepare_tir_view(&view, TemplatePreparationMode::Value)?.facts)
}

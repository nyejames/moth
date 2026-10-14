//! Const value resolution logic.
//!
//! WHAT: evaluates whether an AST expression resolves to a compile-time constant
//!       by substituting known const references and reusing the existing constant folder.
//! WHY: one shared resolver avoids duplicating fold/reference logic across config,
//!      AST finalization, and HIR metadata.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldError, fold_expression_with_constants,
};
use crate::compiler_frontend::ast::const_values::facts::ConstFactValueKind;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
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
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

/// Folded lexical bindings share the phase's canonical content owner.
#[derive(Clone, Debug, Default)]
pub struct ConstValueEnvironment {
    module: Rc<FxHashMap<PathId, ConstValueId>>,
    local: FxHashMap<PathId, ConstValueId>,
}

impl ConstValueEnvironment {
    /// Build an environment over a module's authored constants.
    pub(crate) fn with_module_base(module: FxHashMap<PathId, ConstValueId>) -> Self {
        Self {
            module: Rc::new(module),
            local: FxHashMap::default(),
        }
    }

    /// Insert a resolved const binding introduced by the current lexical scope.
    ///
    /// A local binding shadows a module constant of the same path, which is what
    /// [`Self::module_constant`] relies on being consulted second.
    pub(crate) fn insert(&mut self, path: PathId, value: ConstValueId) {
        self.local.insert(path, value);
    }

    /// Look up a binding introduced by this scope or an enclosing one.
    pub(crate) fn lookup_local(&self, path: &PathId) -> Option<ConstValueId> {
        self.local.get(path).copied()
    }

    /// Look up an authored module constant by path.
    pub(crate) fn module_constant(&self, path: &PathId) -> Option<ConstValueId> {
        self.module.get(path).copied()
    }

    /// Number of bindings a scope copy actually duplicates.
    pub(crate) fn len(&self) -> usize {
        self.local.len()
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

/// The advisory declaration result retains content identity, never a duplicate expression.
#[derive(Debug)]
pub(crate) struct ResolvedConstDeclaration {
    pub(crate) value: ConstValueId,
    pub(crate) value_kind: ConstFactValueKind,
}

pub(crate) struct ConstValueResolver<'a> {
    string_table: &'a mut StringTable,
    const_values: &'a mut ConstValueStore,
    expression_builder: &'a mut AstExpressionBuilder,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
    type_environment: &'a TypeEnvironment,
    // Reuse heap traversal storage across resolutions, including after advisory failures.
    pending_references: Vec<ExprId>,
}

impl<'a> ConstValueResolver<'a> {
    pub(crate) fn new(
        string_table: &'a mut StringTable,
        const_values: &'a mut ConstValueStore,
        expression_builder: &'a mut AstExpressionBuilder,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
        type_environment: &'a TypeEnvironment,
    ) -> Self {
        Self {
            string_table,
            const_values,
            expression_builder,
            template_ir_store,
            type_environment,
            pending_references: Vec::new(),
        }
    }

    pub(crate) fn builder(&self) -> &AstExpressionBuilder {
        self.expression_builder
    }
    pub(crate) fn builder_mut(&mut self) -> &mut AstExpressionBuilder {
        self.expression_builder
    }
    pub(crate) fn const_values(&self) -> &ConstValueStore {
        self.const_values
    }

    pub(crate) fn resolve_private_top_level_declaration(
        &mut self,
        declaration: &Declaration,
        environment: &ConstValueEnvironment,
    ) -> Result<ResolvedConstDeclaration, ConstResolutionError> {
        if declaration.value_mode.is_mutable() {
            return Err(ConstResolutionError::MutableDeclaration);
        }
        let initializer = declaration
            .value
            .initializer()
            .ok_or(ConstResolutionError::NonConstExpression)?;
        let value = self.resolve_expression(initializer, environment)?;
        let metadata = self
            .const_values
            .metadata(value)
            .ok_or(ConstResolutionError::NonConstExpression)?;
        Ok(ResolvedConstDeclaration {
            value,
            value_kind: ConstFactValueKind::from_const_value_kind(metadata.value_kind),
        })
    }

    pub(crate) fn resolve_body_local_declaration(
        &mut self,
        declaration: &Declaration,
        environment: &ConstValueEnvironment,
    ) -> Result<ResolvedConstDeclaration, ConstResolutionError> {
        self.resolve_private_top_level_declaration(declaration, environment)
    }

    pub(crate) fn resolve_expression(
        &mut self,
        expression: ExprId,
        environment: &ConstValueEnvironment,
    ) -> Result<ConstValueId, ConstResolutionError> {
        fn validate_references(
            mut expression: ExpressionView<'_>,
            environment: &ConstValueEnvironment,
            pending: &mut Vec<ExprId>,
        ) -> Result<(), ConstResolutionError> {
            let store = expression.store();
            loop {
                if let ExpressionOperationView::Reference(path) = expression.operation()
                    && environment
                        .lookup_local(&path)
                        .or_else(|| environment.module_constant(&path))
                        .is_none()
                {
                    return Err(ConstResolutionError::UnresolvedReference);
                }
                let children_start = pending.len();
                expression
                    .visit_child_ids(&mut |child| pending.push(child))
                    .map_err(|error| {
                        ConstResolutionError::TemplateClassification(TemplateError::Infrastructure(
                            Box::new(error),
                        ))
                    })?;
                // Reverse only newly scheduled children to retain authored depth-first order.
                pending[children_start..].reverse();
                let Some(next) = pending.pop() else {
                    return Ok(());
                };
                expression = store.view(next).map_err(|error| {
                    ConstResolutionError::TemplateClassification(TemplateError::Infrastructure(
                        Box::new(error),
                    ))
                })?;
            }
        }
        self.pending_references.clear();
        let view = self.expression_builder.view(expression).map_err(|error| {
            ConstResolutionError::TemplateClassification(TemplateError::Infrastructure(Box::new(
                error,
            )))
        })?;
        validate_references(view, environment, &mut self.pending_references)?;
        let resolved = fold_expression_with_constants(
            expression,
            self.expression_builder,
            self.const_values,
            self.string_table,
            self.type_environment,
            |path| {
                environment
                    .lookup_local(&path)
                    .or_else(|| environment.module_constant(&path))
            },
        )
        .map_err(|error| match error {
            // Builder capacity stays on the template lane: runtime classification would hide it.
            ConstantFoldError::Diagnostic(diagnostic)
                if diagnostic.is_compiler_capacity_exceeded() =>
            {
                ConstResolutionError::TemplateClassification(TemplateError::Diagnostic(diagnostic))
            }
            ConstantFoldError::Diagnostic(_) => ConstResolutionError::NonFoldableRuntimeExpression,
            ConstantFoldError::Infrastructure(error) => {
                ConstResolutionError::TemplateClassification(TemplateError::Infrastructure(error))
            }
        })?;
        let view = self.expression_builder.view(resolved).map_err(|error| {
            ConstResolutionError::TemplateClassification(TemplateError::Infrastructure(Box::new(
                error,
            )))
        })?;
        match view.operation() {
            ExpressionOperationView::Constant(value) => {
                self.const_values
                    .value(value)
                    .ok_or(ConstResolutionError::NonConstExpression)?;
                Ok(value)
            }
            ExpressionOperationView::Reference(path) => {
                let value = environment
                    .lookup_local(&path)
                    .or_else(|| environment.module_constant(&path))
                    .ok_or(ConstResolutionError::UnresolvedReference)?;
                self.const_values
                    .value(value)
                    .ok_or(ConstResolutionError::NonConstReference)?;
                Ok(value)
            }
            ExpressionOperationView::Template(template) => {
                classify_template_from_effective_tir(
                    template,
                    &self.template_ir_store,
                    self.expression_builder,
                    self.const_values,
                )?;
                Err(ConstResolutionError::NonConstExpression)
            }
            ExpressionOperationView::Call(_)
            | ExpressionOperationView::MethodCall { .. }
            | ExpressionOperationView::CollectionBuiltin { .. }
            | ExpressionOperationView::MapBuiltin { .. } => {
                Err(ConstResolutionError::CallInConstContext)
            }
            _ => Err(ConstResolutionError::NonConstExpression),
        }
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
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
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
    let preparation = prepare_tir_view(
        &view,
        expression_builder,
        const_values,
        TemplatePreparationMode::Value,
    )?;
    Ok(preparation.facts.final_value_kind)
}

#[cfg(test)]
pub(crate) fn prepare_template_tir_facts(
    template: &Template,
    store: &Rc<RefCell<TemplateIrStore>>,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
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
    Ok(prepare_tir_view(
        &view,
        expression_builder,
        const_values,
        TemplatePreparationMode::Value,
    )?
    .facts)
}

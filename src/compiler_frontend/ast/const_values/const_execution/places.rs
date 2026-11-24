//! Interpreted places, collection mutation and fallible handler execution.
//!
//! WHAT: resolves aliases and place projections, applies projected constant writes, executes
//!       collection reads and mutations, and runs prepared catch handlers.
//! WHY: these operations read or mutate the interpreter's live binding state. Keeping them in one
//!      sibling module leaves the frame machine to sequence instructions while this module owns
//!      place semantics.
//! MUST NOT: compile bodies, decide candidate eligibility or own budget limits.

use super::instructions::ConstInstruction;
use super::labels;
use super::state::{ConstInterpreter, ExecutionState, FrameBindings, FrameOutcome};
use super::{
    AliasTarget, ResolvedPlaceProjection,
    budget::charge_new_value_text,
    values::{
        ProjectionWrite, assign_constant_projections, constant_index, fixed_scalar_is_negative,
        project_constant_value, unsupported_error,
    },
};
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValue, ConstValueField, ConstValueId, ConstValueMetadata,
    ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, PlaceId, PlaceProjectionView,
    SuccessShape,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeDeclarationRef;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use rustc_hash::FxHashMap;
use std::rc::Rc;

/// One receiver resolved to a bound collection value.
///
/// Contents are read separately: statement mutation charges and copies them only after argument
/// evaluation, while expression removal does so before index evaluation.
struct ResolvedCollection {
    root: PathId,
    projections: Vec<ResolvedPlaceProjection>,
    root_value: ConstValueId,
    current: ConstValueId,
}

impl ResolvedCollection {
    fn into_write(self, metadata: ConstValueMetadata) -> CollectionWrite {
        CollectionWrite {
            root: self.root,
            projections: self.projections,
            root_value: self.root_value,
            metadata,
        }
    }
}

/// The target fields needed to write changed collection contents back.
struct CollectionWrite {
    root: PathId,
    projections: Vec<ResolvedPlaceProjection>,
    root_value: ConstValueId,
    metadata: ConstValueMetadata,
}

/// Whether a mutation arm changed contents or consumed the statement through a handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MutationOutcome {
    Changed,
    Consumed,
}

/// What a mutation receiver resolved to.
enum CollectionTarget {
    /// A bound collection value ready for its contents to be read.
    Resolved(ResolvedCollection),
    /// The receiver is not a binding, or names no bound value.
    NotBound,
    /// The receiver names a binding whose projection does not resolve.
    InvalidProjection,
}

/// Removes fallible handling wrappers so a successful inner operation can be evaluated.
pub(super) fn unwrap_fallible_expression(
    mut expression: ExprId,
    builder: &AstExpressionBuilder,
) -> Result<ExprId, ConstantFoldError> {
    loop {
        match builder.view(expression)?.operation() {
            ExpressionOperationView::HandleFallible { value, .. } => expression = value.id(),
            ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) => {
                expression = block.handled_value;
            }
            _ => return Ok(expression),
        }
    }
}

/// Resolves the prepared catch body selected by one catch expression.
///
/// The authored expression installs its prepared body before evaluating any of its children.
pub(super) fn fallible_handler_instructions(
    builder: &AstExpressionBuilder,
    state: &ExecutionState,
    expression: ExprId,
) -> Result<Option<Rc<[ConstInstruction]>>, ConstantFoldError> {
    let view = builder.view(expression)?;
    let is_handler = match view.operation() {
        ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) => {
            matches!(block.handler, FallibleHandling::Handler { .. })
        }
        _ => false,
    };
    if !is_handler {
        return Ok(None);
    }
    if let Some(body) = state
        .handler_bodies
        .as_ref()
        .and_then(|bodies| bodies.get(&expression))
        .cloned()
    {
        return Ok(Some(body));
    }
    if state.handler_bodies.is_none() {
        // Ordinary expression rewriting has no active const frame. It never executes the handler,
        // so there is nothing to install.
        return Ok(None);
    }
    Err(InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "const executor selected a catch body that declaration preparation did not compile",
    )
    .into())
}

/// Returns the optional error binding from a selected catch handler.
pub(super) fn fallible_handler_error_binding(
    builder: &AstExpressionBuilder,
    expression: ExprId,
) -> Result<Option<PathId>, ConstantFoldError> {
    Ok(match builder.view(expression)?.operation() {
        ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) => match &block.handler {
            FallibleHandling::Handler { error, .. } => {
                error.as_ref().map(|binding| binding.error_binding)
            }
            FallibleHandling::Propagate => None,
        },
        _ => None,
    })
}

/// Resolve a mutable binding's direct alias target, if its initializer is an unprojected
/// reference or a field projection from a bound place.
///
/// `copy place` is an independent value; only a shared reference aliases its source.
pub(super) fn direct_alias_target(
    expression: ExprId,
    builder: &AstExpressionBuilder,
    bindings: &FrameBindings,
) -> Result<Option<(PathId, Vec<ResolvedPlaceProjection>)>, ConstantFoldError> {
    let mut field_suffix = Vec::new();
    let mut current = expression;
    loop {
        match builder.view(current)?.operation() {
            ExpressionOperationView::Reference(path) if bindings.contains_key(&path) => {
                field_suffix.reverse();
                return Ok(Some((path, field_suffix)));
            }
            ExpressionOperationView::FieldAccess { base, field } => {
                field_suffix.push(ResolvedPlaceProjection::Field(field));
                current = base.id();
            }
            _ => return Ok(None),
        }
    }
}

/// Prepend an existing alias target's projections and canonical root to a new place path.
pub(super) fn expand_alias_target(
    mut root: PathId,
    mut projections: Vec<ResolvedPlaceProjection>,
    aliases: &FxHashMap<PathId, AliasTarget>,
) -> (PathId, Vec<ResolvedPlaceProjection>) {
    for _ in 0..64 {
        let Some(target) = aliases.get(&root) else {
            break;
        };
        if target.root == root {
            break;
        }
        let mut combined = target.projections.clone();
        combined.append(&mut projections);
        projections = combined;
        root = target.root;
    }
    (root, projections)
}

/// Follow the active frame's alias chain to the canonical binding root.
fn canonical_alias_root(mut root: PathId, aliases: &FxHashMap<PathId, AliasTarget>) -> PathId {
    // Alias chains are created from already canonical roots, but a small fixed walk keeps
    // malformed state from becoming an unbounded loop.
    for _ in 0..64 {
        let Some(next) = aliases.get(&root) else {
            break;
        };
        if next.root == root {
            break;
        }
        root = next.root;
    }
    root
}

/// Resolves an expression that names an already-bound constant value without producing a new one.
fn bound_constant_value(
    expression: ExprId,
    builder: &AstExpressionBuilder,
    bindings: &FrameBindings,
) -> Result<Option<ConstValueId>, ConstantFoldError> {
    Ok(match builder.view(expression)?.operation() {
        ExpressionOperationView::Constant(value) => Some(value),
        ExpressionOperationView::Reference(path) => bindings.get(&path).copied(),
        ExpressionOperationView::Copy(place) if place.projections().len() == 0 => {
            bindings.get(&place.metadata().root).copied()
        }
        _ => None,
    })
}

impl ConstInterpreter<'_> {
    /// Write one root value and refresh every binding that aliases that root.
    pub(super) fn write_binding_value(
        &mut self,
        root: PathId,
        value: ConstValueId,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        // Refreshing every alias is part of the write's work. Charge the scan before mutating so a
        // function with many aliases and repeated writes cannot spend quadratic time off-budget.
        let alias_count = self.state.aliases.len();
        for _ in 0..alias_count {
            self.budget.step()?;
        }
        bindings.insert(root, value);
        for (alias, target) in &self.state.aliases {
            if canonical_alias_root(target.root, &self.state.aliases) == root
                && let Some(projected) = project_constant_value(
                    self.constants,
                    value,
                    &target.projections,
                    self.path_fork,
                )
            {
                bindings.insert(*alias, projected);
            }
        }
        Ok(())
    }

    /// Builds the caught `Error` record that a handler error binding refers to.
    fn caught_error_value(
        &mut self,
        code: BuiltinErrorCode,
        message: Option<String>,
        span: Option<SourceSpan>,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let error_name = self.string_table.intern("Error");
        let Some(reference) = self.scope.get_reference(&error_name) else {
            return Ok(None);
        };
        let error_type = match reference {
            ScopeDeclarationRef::Local { declaration, .. } => declaration.type_id,
            ScopeDeclarationRef::Shared(declaration) => declaration.type_id,
            _ => return Ok(None),
        };
        let code_name = self.string_table.intern("code");
        let Some(code_field) = self.types.field_for(error_type, code_name, self.path_fork) else {
            return Ok(None);
        };
        let message_name = self.string_table.intern("message");
        let Some(message_field) = self
            .types
            .field_for(error_type, message_name, self.path_fork)
        else {
            return Ok(None);
        };
        let scalar = moth_lexical::numeric::fixed_scalar::FixedScalarValue::unsigned(
            moth_lexical::numeric::fixed_scalar::FixedScalar::U32,
            code.as_u32().into(),
        )
        .ok_or_else(|| {
            unsupported_error(labels::COLLECTION_ERROR_VALUE, span, self.string_table)
        })?;
        let code_value = self.constants.insert_resolved(
            code_field.type_id,
            ConstValuePayload::FixedScalar(scalar),
            span,
        )?;
        let message = match message {
            Some(message) => self.string_table.get_or_intern(message),
            None => self.string_table.intern(code.default_message()),
        };
        let message_value = self.constants.insert_resolved(
            message_field.type_id,
            ConstValuePayload::String(ConstStringValue::Text(message)),
            span,
        )?;
        Ok(Some(self.constants.insert_resolved(
            error_type,
            ConstValuePayload::Record(vec![
                ConstValueField {
                    name: message_field.name,
                    value: message_value,
                },
                ConstValueField {
                    name: code_field.name,
                    value: code_value,
                },
            ]),
            span,
        )?))
    }

    fn execute_collection_handler(
        &mut self,
        error_code: BuiltinErrorCode,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        self.execute_builtin_handler(error_code, None, self.budget.span, bindings)
    }

    /// Run the active handler for a discovered builtin failure. The receiving site owns delivery.
    pub(super) fn execute_builtin_handler(
        &mut self,
        error_code: BuiltinErrorCode,
        message: Option<String>,
        span: Option<SourceSpan>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let active = self.state.active_handler.clone();
        let Some(handler_body) = active.body else {
            return Ok(());
        };
        let value_count = self.constants.value_count();
        if let Some(error_binding) = active.error_binding
            && let Some(error) = self.caught_error_value(error_code, message, span)?
        {
            bindings.insert(error_binding, error);
        }
        // Handler execution is isolated: it cannot re-enter itself.
        let outcome = self.execute_handler_body(&handler_body, bindings)?;
        charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        self.state.outcome = Some(outcome);
        Ok(())
    }

    /// Evaluates non-mutating collection reads that have one ordinary success value.
    pub(super) fn evaluate_collection_read(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let inner = unwrap_fallible_expression(expression, self.builder)?;
        let (operation, receiver, arguments, span) = {
            let view = self.builder.view(inner)?;
            let ExpressionOperationView::CollectionBuiltin {
                receiver,
                facts,
                arguments,
            } = view.operation()
            else {
                return Ok(None);
            };
            (
                facts.operation,
                receiver,
                arguments
                    .iter()
                    .map(|argument| argument.value().id())
                    .collect::<Vec<_>>(),
                view.span(),
            )
        };
        if operation == CollectionBuiltinOp::Remove {
            return self.evaluate_collection_remove(
                expression,
                receiver.id(),
                &arguments,
                bindings,
            );
        }
        if !matches!(
            operation,
            CollectionBuiltinOp::Length | CollectionBuiltinOp::Get
        ) {
            return Ok(None);
        }
        let Some(collection) = bound_constant_value(receiver.id(), self.builder, bindings)? else {
            return Ok(None);
        };
        let Some(ConstValuePayload::Collection(items)) = self.constants.payload(collection) else {
            return Ok(None);
        };
        let result_type = match self.builder.view(expression)?.success() {
            SuccessShape::One(type_id) => type_id,
            _ => return Ok(None),
        };
        match operation {
            CollectionBuiltinOp::Length => {
                let length = i32::try_from(items.len()).map_err(|_| {
                    unsupported_error(labels::COLLECTION_LENGTH, span, self.string_table)
                })?;
                let scalar = moth_lexical::numeric::fixed_scalar::FixedScalarValue::signed(
                    moth_lexical::numeric::fixed_scalar::FixedScalar::I32,
                    i64::from(length),
                )
                .ok_or_else(|| {
                    unsupported_error(labels::COLLECTION_LENGTH, span, self.string_table)
                })?;
                Ok(Some(self.constants.insert_resolved(
                    result_type,
                    ConstValuePayload::FixedScalar(scalar),
                    span,
                )?))
            }
            CollectionBuiltinOp::Get => {
                self.evaluate_collection_get(collection, arguments.as_slice(), bindings)
            }
            _ => Ok(None),
        }
    }

    /// Index one bound collection, running the selected handler for a non-addressable index.
    fn evaluate_collection_get(
        &mut self,
        collection: ConstValueId,
        arguments: &[ExprId],
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let [index] = arguments else {
            return Ok(None);
        };
        let Some(index_value) = self.evaluate_expression(*index, bindings)? else {
            return Ok(None);
        };
        // Stored values are immutable, so keep the receiver's ID across index evaluation instead
        // of copying its contents. A write through an alias creates a different value ID.
        let Some(ConstValuePayload::Collection(items)) = self.constants.payload(collection) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const collection read lost its receiver value",
            )
            .into());
        };
        let index = constant_index(self.constants, index_value);
        if collection_index_is_out_of_bounds(self.constants, index, index_value, items.len()) {
            self.execute_collection_handler(
                BuiltinErrorCode::CollectionIndexOutOfBounds,
                bindings,
            )?;
            return Ok(None);
        }
        let Some(index) = index else {
            return Ok(None);
        };
        Ok(items.get(index).copied())
    }

    /// Removes one collection item, returns it, and writes the shrunk collection back.
    fn evaluate_collection_remove(
        &mut self,
        expression: ExprId,
        receiver: ExprId,
        arguments: &[ExprId],
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let target = match self.resolve_collection_target(receiver, bindings)? {
            CollectionTarget::Resolved(target) => target,
            CollectionTarget::NotBound | CollectionTarget::InvalidProjection => return Ok(None),
        };
        let error_span = self
            .builder
            .view(expression)
            .ok()
            .and_then(|view| view.span());
        let metadata = self
            .constants
            .metadata(target.current)
            .cloned()
            .ok_or_else(|| {
                unsupported_error(labels::COLLECTION_REMOVAL, error_span, self.string_table)
            })?;
        let items = match self.constants.payload(target.current) {
            Some(ConstValuePayload::Collection(items)) => items,
            _ => return Ok(None),
        };
        // Expression removal charges the projected copy before its index is evaluated, even when
        // that index later recovers through a handler.
        charge_projected_items(self.budget, items.len(), metadata.span)?;
        let mut items = items.clone();
        let [index] = arguments else {
            return Ok(None);
        };
        let Some(index_value) = self.evaluate_expression(*index, bindings)? else {
            return Ok(None);
        };
        let index = constant_index(self.constants, index_value);
        if collection_index_is_out_of_bounds(self.constants, index, index_value, items.len()) {
            if self.state.active_handler.body.is_none() {
                return Ok(None);
            }
            self.budget.retain_values(items.len(), metadata.span)?;
            self.execute_collection_handler(
                BuiltinErrorCode::CollectionIndexOutOfBounds,
                bindings,
            )?;
            return Ok(None);
        }
        let Some(index) = index else {
            return Ok(None);
        };
        Ok(Some(self.remove_collection_item(
            target, metadata, &mut items, index, bindings,
        )?))
    }

    /// Resolve a mutation receiver to the bound collection value it names.
    ///
    /// The caller owns what "not bound" and "invalid projection" mean: statement mutation refuses
    /// an invalid projection while expression removal treats it as "not this expression's value".
    fn resolve_collection_target(
        &mut self,
        receiver: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<CollectionTarget, ConstantFoldError> {
        let Some((root, projections)) = self.resolve_mutation_receiver(receiver, bindings)? else {
            return Ok(CollectionTarget::NotBound);
        };
        let root = canonical_alias_root(root, &self.state.aliases);
        let Some(root_value) = bindings.get(&root).copied() else {
            return Ok(CollectionTarget::NotBound);
        };
        let Some(current) =
            project_constant_value(self.constants, root_value, &projections, self.path_fork)
        else {
            return Ok(CollectionTarget::InvalidProjection);
        };
        Ok(CollectionTarget::Resolved(ResolvedCollection {
            root,
            projections,
            root_value,
            current,
        }))
    }

    /// Remove `index` from resolved collection contents and write the shrunk collection back.
    fn remove_collection_item(
        &mut self,
        target: ResolvedCollection,
        metadata: ConstValueMetadata,
        items: &mut Vec<ConstValueId>,
        index: usize,
        bindings: &mut FrameBindings,
    ) -> Result<ConstValueId, ConstantFoldError> {
        let removed = items.remove(index);
        self.write_collection_contents(
            target.into_write(metadata),
            std::mem::take(items),
            bindings,
        )?;
        Ok(removed)
    }

    /// Store updated collection contents, charging the write and refreshing all aliases.
    fn write_collection_contents(
        &mut self,
        write: CollectionWrite,
        items: Vec<ConstValueId>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let span = write.metadata.span;
        self.budget
            .retain_values(items.len().saturating_add(1), span)?;
        self.budget
            .retain_memory_bytes(std::mem::size_of::<ConstValue>(), span)?;
        let precharged_index = self.constants.value_count();
        let updated = self
            .constants
            .insert_value(write.metadata, ConstValuePayload::Collection(items))?;
        self.budget
            .precharged_value_indices
            .insert(precharged_index);
        let mut projected_write = ProjectionWrite {
            constants: &mut *self.constants,
            path_fork: self.path_fork,
            budget: &mut *self.budget,
            string_table: &mut *self.string_table,
        };
        let root_value = assign_constant_projections(
            write.root_value,
            &write.projections,
            updated,
            &mut projected_write,
        )?;
        self.write_binding_value(write.root, root_value, bindings)
    }

    /// Resolves a mutation receiver expression to its bound root and field/index projection path.
    fn resolve_mutation_receiver(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<(PathId, Vec<ResolvedPlaceProjection>)>, ConstantFoldError> {
        let mut field_suffix = Vec::new();
        let mut current = expression;
        loop {
            match self.builder.view(current)?.operation() {
                ExpressionOperationView::Reference(path) => {
                    field_suffix.reverse();
                    let (root, projections) =
                        expand_alias_target(path, field_suffix, &self.state.aliases);
                    return Ok(Some((root, projections)));
                }
                ExpressionOperationView::Copy(place) => {
                    let Some((root, mut projections)) =
                        self.evaluate_place_projections(place.id(), bindings)?
                    else {
                        return Ok(None);
                    };
                    field_suffix.reverse();
                    projections.extend(field_suffix);
                    return Ok(Some(expand_alias_target(
                        root,
                        projections,
                        &self.state.aliases,
                    )));
                }
                ExpressionOperationView::FieldAccess { base, field } => {
                    field_suffix.push(ResolvedPlaceProjection::Field(field));
                    current = base.id();
                }
                _ => return Ok(None),
            }
        }
    }

    /// Executes one collection mutation statement and reports whether the expression was consumed.
    ///
    /// WHAT: supports `push`, `set` and `remove` on a bound collection and rebinds the resulting
    ///       immutable constant.
    /// WHY: mutation is a statement-level effect; the executor must not claim a collection value
    ///      was produced for an evaluation expression that has no ordinary single result.
    pub(super) fn execute_collection_mutation(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<bool, ConstantFoldError> {
        let consumed = self.with_expression_handler(expression, |interpreter| {
            interpreter.try_collection_mutation(expression, bindings)
        })?;
        if consumed && matches!(self.state.outcome, Some(FrameOutcome::Recovered(_))) {
            // Statement recovery consumes the operation even when it produces no values.
            self.state.outcome = None;
        }
        Ok(consumed)
    }

    fn try_collection_mutation(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<bool, ConstantFoldError> {
        let expression = unwrap_fallible_expression(expression, self.builder)?;
        let (operation, receiver, arguments, span) = {
            let view = self.builder.view(expression)?;
            let ExpressionOperationView::CollectionBuiltin {
                receiver,
                facts,
                arguments,
            } = view.operation()
            else {
                return Ok(false);
            };
            let operation = facts.operation;
            if !matches!(
                operation,
                CollectionBuiltinOp::PushGrowable
                    | CollectionBuiltinOp::PushFixed
                    | CollectionBuiltinOp::Set
                    | CollectionBuiltinOp::Remove
            ) {
                return Ok(false);
            }
            (
                operation,
                receiver,
                arguments
                    .iter()
                    .map(|argument| argument.value().id())
                    .collect::<Vec<_>>(),
                view.span(),
            )
        };
        let target = self.resolve_collection_target(receiver.id(), bindings)?;
        if self.state.outcome.is_some() {
            return Ok(true);
        }
        let target = match target {
            CollectionTarget::Resolved(target) => target,
            CollectionTarget::NotBound => return Ok(false),
            CollectionTarget::InvalidProjection => {
                return Err(unsupported_error(
                    labels::COLLECTION_MUTATION,
                    span,
                    self.string_table,
                ));
            }
        };
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let Some(value) = self.evaluate_expression(argument, bindings)? else {
                return Ok(true);
            };
            values.push(value);
        }
        let metadata = self
            .constants
            .metadata(target.current)
            .cloned()
            .ok_or_else(|| {
                unsupported_error(labels::COLLECTION_MUTATION, span, self.string_table)
            })?;
        let items = match self.constants.payload(target.current) {
            Some(ConstValuePayload::Collection(items)) => items,
            _ => {
                return Err(unsupported_error(
                    labels::COLLECTION_MUTATION,
                    span,
                    self.string_table,
                ));
            }
        };
        // Argument evaluation precedes this charge: a handler-recovered argument consumes the
        // statement without copying its contents, so the mutation pays only for copies it writes.
        charge_projected_items(self.budget, items.len(), span)?;
        let mut items = items.clone();
        let outcome = match operation {
            CollectionBuiltinOp::PushGrowable => {
                self.apply_push_growable(&mut items, values.as_slice(), span)?
            }
            CollectionBuiltinOp::PushFixed => {
                let item_count = items.len();
                let pushed = self.apply_push_fixed(
                    values.as_slice(),
                    item_count,
                    metadata.type_id,
                    span,
                    bindings,
                )?;
                if let Some(value) = pushed {
                    items.push(value);
                    MutationOutcome::Changed
                } else {
                    MutationOutcome::Consumed
                }
            }
            CollectionBuiltinOp::Set => {
                let assigned = self.apply_set(values.as_slice(), items.len(), span, bindings)?;
                if let Some((index, value)) = assigned {
                    items[index] = value;
                    MutationOutcome::Changed
                } else {
                    MutationOutcome::Consumed
                }
            }
            CollectionBuiltinOp::Remove => {
                let [index_value] = values.as_slice() else {
                    return Err(unsupported_error(
                        labels::COLLECTION_REMOVAL,
                        span,
                        self.string_table,
                    ));
                };
                let index = constant_index(self.constants, *index_value);
                if collection_index_is_out_of_bounds(
                    self.constants,
                    index,
                    *index_value,
                    items.len(),
                ) {
                    self.handle_collection_bounds_error(items.len(), span, bindings)?;
                    return Ok(true);
                }
                let Some(index) = index else {
                    return Err(unsupported_error(
                        labels::COLLECTION_REMOVAL,
                        span,
                        self.string_table,
                    ));
                };
                self.remove_collection_item(target, metadata, &mut items, index, bindings)?;
                return Ok(true);
            }
            CollectionBuiltinOp::Get | CollectionBuiltinOp::Length => {
                return Ok(false);
            }
        };
        if matches!(outcome, MutationOutcome::Changed) {
            self.write_collection_contents(target.into_write(metadata), items, bindings)?;
        }
        Ok(true)
    }

    fn apply_push_growable(
        &mut self,
        items: &mut Vec<ConstValueId>,
        values: &[ConstValueId],
        span: Option<SourceSpan>,
    ) -> Result<MutationOutcome, ConstantFoldError> {
        let [value] = values else {
            return Err(unsupported_error(
                labels::COLLECTION_MUTATION,
                span,
                self.string_table,
            ));
        };
        items.push(*value);
        Ok(MutationOutcome::Changed)
    }

    fn apply_push_fixed(
        &mut self,
        values: &[ConstValueId],
        item_count: usize,
        collection_type: TypeId,
        span: Option<SourceSpan>,
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let [value] = values else {
            return Err(unsupported_error(
                labels::FIXED_COLLECTION_PUSH,
                span,
                self.string_table,
            ));
        };
        let Some(capacity) = self.types.collection_fixed_capacity(collection_type) else {
            return Err(unsupported_error(
                labels::FIXED_COLLECTION_PUSH,
                span,
                self.string_table,
            ));
        };
        if item_count >= capacity {
            if self.state.active_handler.body.is_none() {
                return Err(unsupported_error(
                    labels::FIXED_COLLECTION_PUSH,
                    span,
                    self.string_table,
                ));
            }
            self.budget.retain_values(item_count, span)?;
            self.execute_collection_handler(
                BuiltinErrorCode::CollectionFixedCapacityExceeded,
                bindings,
            )?;
            return Ok(None);
        }
        Ok(Some(*value))
    }

    fn apply_set(
        &mut self,
        values: &[ConstValueId],
        item_count: usize,
        span: Option<SourceSpan>,
        bindings: &mut FrameBindings,
    ) -> Result<Option<(usize, ConstValueId)>, ConstantFoldError> {
        let [index_value, value] = values else {
            return Err(unsupported_error(
                labels::COLLECTION_MUTATION,
                span,
                self.string_table,
            ));
        };
        let index = constant_index(self.constants, *index_value);
        if collection_index_is_out_of_bounds(self.constants, index, *index_value, item_count) {
            self.handle_collection_bounds_error(item_count, span, bindings)?;
            return Ok(None);
        }
        let Some(index) = index else {
            return Err(unsupported_error(
                labels::COLLECTION_MUTATION,
                span,
                self.string_table,
            ));
        };
        Ok(Some((index, *value)))
    }

    /// Report a collection bounds failure through the selected handler.
    fn handle_collection_bounds_error(
        &mut self,
        item_count: usize,
        span: Option<SourceSpan>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        if self.state.active_handler.body.is_none() {
            return Err(unsupported_error(
                labels::COLLECTION_BOUNDS_ERROR,
                span,
                self.string_table,
            ));
        }
        self.budget.retain_values(item_count, span)?;
        self.execute_collection_handler(BuiltinErrorCode::CollectionIndexOutOfBounds, bindings)
    }

    /// Resolves one place's field/index projections to index constants before value-graph updates.
    pub(super) fn evaluate_place_projections(
        &mut self,
        place: PlaceId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<(PathId, Vec<ResolvedPlaceProjection>)>, ConstantFoldError> {
        let (root, projections) = {
            let place_view = self.builder.place(place)?;
            let projections = place_view
                .projections()
                .map(|projection| match projection {
                    PlaceProjectionView::Field { field, .. } => Ok((Some(field), None)),
                    PlaceProjectionView::Index { index, .. } => Ok((None, Some(index.id()))),
                })
                .collect::<Result<Vec<_>, InfrastructureFailure>>()?;
            (place_view.metadata().root, projections)
        };
        let mut resolved = Vec::with_capacity(projections.len());
        for (field, index) in projections {
            match (field, index) {
                (Some(field), _) => {
                    resolved.push(ResolvedPlaceProjection::Field(field));
                }
                (_, Some(index)) => {
                    let Some(value) = self.evaluate_expression(index, bindings)? else {
                        return Ok(None);
                    };
                    let Some(index) = constant_index(self.constants, value) else {
                        return Err(unsupported_error(
                            labels::PLACE_PROJECTION,
                            self.builder.view(index).ok().and_then(|view| view.span()),
                            self.string_table,
                        ));
                    };
                    resolved.push(ResolvedPlaceProjection::Index(index));
                }
                (None, None) => unreachable!("a place projection is a field or an index"),
            }
        }
        Ok(Some((root, resolved)))
    }
}

/// Charge the copy-on-write slot vector a collection operation is about to clone.
///
/// WHAT: the clone is physical scratch allocation that the interpreter deliberately treats as
///       retained attempt memory, not merely as a temporary borrow of the constant graph.
/// WHY: each caller owns the charge point its contract requires. Removal charges before its index
///      is evaluated, even when that index later recovers through a handler; statement mutation
///      charges after argument evaluation, so a recovered statement pays only for copies it
///      writes. The budget is a separate field from the constant store, so this helper keeps the
///      payload borrow alive while charging.
fn charge_projected_items(
    budget: &mut super::budget::ExecutionBudget,
    item_count: usize,
    span: Option<SourceSpan>,
) -> Result<(), ConstantFoldError> {
    budget.retain_memory_bytes(
        item_count
            .saturating_add(1)
            .saturating_mul(std::mem::size_of::<ConstValueId>()),
        span,
    )
}

/// True when a resolved collection index cannot address the collection.
fn collection_index_is_out_of_bounds(
    constants: &ConstValueStore,
    index: Option<usize>,
    index_value: ConstValueId,
    item_count: usize,
) -> bool {
    match index {
        Some(index) => index >= item_count,
        None => fixed_scalar_is_negative(constants, index_value),
    }
}

//! Interpreted places, collection mutation and fallible handler execution.
//!
//! WHAT: resolves live place projections, executes collection operations, and runs catch handlers.
//! WHY: these operations read or mutate the interpreter's live binding state. Keeping them in one
//!      sibling module leaves the frame machine to sequence instructions while this module owns
//!      place semantics.
//! MUST NOT: compile bodies, decide candidate eligibility or own budget limits.

use super::instructions::ConstInstruction;
use super::labels;
use super::state::{ConstInterpreter, ExecutionState, FrameBindings, FrameOutcome};
use super::storage::{LivePlace, LiveValue, Permission};
use super::{
    ResolvedPlaceProjection,
    budget::charge_new_value_text,
    values::{constant_index, fixed_scalar_is_negative, unsupported_error},
};
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueField, ConstValueId, ConstValuePayload, ConstValueStore,
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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use std::rc::Rc;

fn collection_mutation_arity() -> ConstantFoldError {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "typed collection mutation lost its argument slots",
    )
    .into()
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

impl ConstInterpreter<'_> {
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
            let value = self.materialize_value(error, Permission::ReadOnly)?;
            let binding = self
                .state
                .storage
                .bind_slot(value, Permission::ReadOnly, self.budget)?;
            bindings.insert(error_binding, binding);
        }
        // Handler execution is isolated: it cannot re-enter itself.
        let outcome = self.execute_handler_body(&handler_body, bindings)?;
        charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        self.state.outcome = Some(outcome);
        Ok(())
    }

    /// Resolve authored local references and field chains without evaluating them.
    pub(super) fn resolve_live_place(
        &mut self,
        expression: ExprId,
        bindings: &FrameBindings,
    ) -> Result<Option<(PathId, LivePlace)>, ConstantFoldError> {
        let mut fields = Vec::new();
        let mut current = expression;
        loop {
            match self.builder.view(current)?.operation() {
                ExpressionOperationView::Reference(root) => {
                    let Some(binding) = bindings.get(&root) else {
                        return Ok(None);
                    };
                    fields.reverse();
                    let span = self.builder.view(expression)?.span();
                    return Ok(self
                        .project_live_place(binding.place(), &fields, span)?
                        .map(|place| (root, place)));
                }
                ExpressionOperationView::FieldAccess { base, field } => {
                    fields.push(ResolvedPlaceProjection::Field(field));
                    current = base.id();
                }
                _ => return Ok(None),
            }
        }
    }

    pub(super) fn project_live_place(
        &mut self,
        mut place: LivePlace,
        projections: &[ResolvedPlaceProjection],
        span: Option<SourceSpan>,
    ) -> Result<Option<LivePlace>, ConstantFoldError> {
        for projection in projections {
            let value = self.read_place(place, span)?;
            let Some(projected) = self.project_live_value(value, *projection, span)? else {
                return Ok(None);
            };
            place = projected;
        }
        Ok(Some(place))
    }

    /// Project an evaluated value, preserving witnesses through coercion edges.
    pub(super) fn project_live_value(
        &mut self,
        mut value: LiveValue,
        projection: ResolvedPlaceProjection,
        span: Option<SourceSpan>,
    ) -> Result<Option<LivePlace>, ConstantFoldError> {
        self.validate_live_value(value, span)?;
        while matches!(
            self.constants.payload(value.shape),
            Some(ConstValuePayload::Coerced(_))
        ) {
            let Some(inner) = self.state.storage.project(value, 0)? else {
                return Ok(None);
            };
            value = self.read_place(inner, span)?;
        }
        let index = match projection {
            ResolvedPlaceProjection::Index(index) => index,
            ResolvedPlaceProjection::Field(field) => {
                let fields = match self.constants.payload(value.shape) {
                    Some(ConstValuePayload::Record(fields))
                    | Some(ConstValuePayload::Choice { fields, .. }) => fields,
                    _ => return Ok(None),
                };
                let Some(index) = fields
                    .iter()
                    .position(|entry| self.path_fork.component(entry.name) == Some(field))
                else {
                    return Ok(None);
                };
                index
            }
        };
        self.state.storage.project(value, index)
    }

    pub(super) fn evaluate_collection_read(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
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
            let mut iter = arguments.iter();
            let first = iter.next().map(|argument| argument.value().id());
            let single = if iter.next().is_none() { first } else { None };
            (facts.operation, receiver.id(), single, view.span())
        };
        if operation == CollectionBuiltinOp::Remove {
            return self.evaluate_collection_remove(receiver, arguments, span, bindings);
        }
        if !matches!(
            operation,
            CollectionBuiltinOp::Length | CollectionBuiltinOp::Get
        ) {
            return Ok(None);
        }
        let Some(collection) = self.evaluate_expression(receiver, bindings)? else {
            return Ok(None);
        };
        let Some(allocation) = collection.allocation else {
            return Ok(None);
        };
        match operation {
            CollectionBuiltinOp::Length => {
                let result_type = match self.builder.view(expression)?.success() {
                    SuccessShape::One(type_id) => type_id,
                    _ => return Ok(None),
                };
                self.validate_live_value(collection, span)?;
                let length =
                    i32::try_from(self.state.storage.children(allocation).len()).map_err(|_| {
                        unsupported_error(labels::COLLECTION_LENGTH, span, self.string_table)
                    })?;
                let scalar = moth_lexical::numeric::fixed_scalar::FixedScalarValue::signed(
                    moth_lexical::numeric::fixed_scalar::FixedScalar::I32,
                    i64::from(length),
                )
                .ok_or_else(|| {
                    unsupported_error(labels::COLLECTION_LENGTH, span, self.string_table)
                })?;
                let shape = self.constants.insert_resolved(
                    result_type,
                    ConstValuePayload::FixedScalar(scalar),
                    span,
                )?;
                Ok(Some(LiveValue::leaf(shape, Permission::Writable)))
            }
            CollectionBuiltinOp::Get => {
                let Some(index) = arguments else {
                    return Ok(None);
                };
                let Some(value) = self.evaluate_expression(index, bindings)? else {
                    return Ok(None);
                };
                self.validate_live_value(collection, span)?;
                let index = constant_index(self.constants, value.shape);
                let count = self.state.storage.children(allocation).len();
                if collection_index_is_out_of_bounds(self.constants, index, value.shape, count) {
                    self.execute_collection_handler(
                        BuiltinErrorCode::CollectionIndexOutOfBounds,
                        bindings,
                    )?;
                    return Ok(None);
                }
                let Some(index) = index else {
                    return Ok(None);
                };
                let Some(place) = self.state.storage.project(collection.read_only(), index)? else {
                    return Ok(None);
                };
                Ok(Some(self.read_place(place, span)?.read_only()))
            }
            _ => Ok(None),
        }
    }

    fn mutable_collection_receiver(
        &mut self,
        receiver: ExprId,
        bindings: &FrameBindings,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        let Some((root, place)) = self.resolve_live_place(receiver, bindings)? else {
            return Ok(None);
        };
        let span = self.builder.view(receiver)?.span();
        let value = self.read_place(place, span)?;
        if value.permission == Permission::ReadOnly {
            return Err(self.mutable_access_error(root, span));
        }
        Ok(Some(value))
    }

    fn evaluate_collection_remove(
        &mut self,
        receiver: ExprId,
        argument: Option<ExprId>,
        span: Option<SourceSpan>,
        bindings: &mut FrameBindings,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        let Some(collection) = self.mutable_collection_receiver(receiver, bindings)? else {
            return Ok(None);
        };
        let Some(allocation) = collection.allocation else {
            return Ok(None);
        };
        let Some(index) = argument else {
            return Ok(None);
        };
        let Some(value) = self.evaluate_expression(index, bindings)? else {
            return Ok(None);
        };
        self.validate_live_value(collection, span)?;
        let index = constant_index(self.constants, value.shape);
        let count = self.state.storage.children(allocation).len();
        if collection_index_is_out_of_bounds(self.constants, index, value.shape, count) {
            if self.state.active_handler.body.is_some() {
                self.execute_collection_handler(
                    BuiltinErrorCode::CollectionIndexOutOfBounds,
                    bindings,
                )?;
            }
            return Ok(None);
        }
        let Some(index) = index else {
            return Ok(None);
        };
        self.state.storage.remove(allocation, index, self.budget)
    }

    pub(super) fn execute_collection_mutation(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<bool, ConstantFoldError> {
        let consumed = self.with_expression_handler(expression, |interpreter| {
            interpreter.try_collection_mutation(expression, bindings)
        })?;
        if consumed && matches!(self.state.outcome, Some(FrameOutcome::Recovered(_))) {
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
            let arity = match facts.operation {
                CollectionBuiltinOp::Set => 2,
                CollectionBuiltinOp::PushGrowable
                | CollectionBuiltinOp::PushFixed
                | CollectionBuiltinOp::Remove => 1,
                _ => return Ok(false),
            };
            if arguments.len() != arity {
                return Err(collection_mutation_arity());
            }
            let mut iter = arguments.iter().map(|argument| argument.value().id());
            (
                facts.operation,
                receiver.id(),
                [iter.next(), iter.next()],
                view.span(),
            )
        };
        let Some(collection) = self.mutable_collection_receiver(receiver, bindings)? else {
            return Ok(false);
        };
        let Some(allocation) = collection.allocation else {
            return Err(unsupported_error(
                labels::COLLECTION_MUTATION,
                span,
                self.string_table,
            ));
        };
        let mut values = [None, None];
        for (slot, argument) in values.iter_mut().zip(arguments.into_iter().flatten()) {
            let Some(value) = self.evaluate_expression(argument, bindings)? else {
                return Ok(true);
            };
            *slot = Some(value);
        }
        self.budget.span = span;
        self.validate_live_value(collection, span)?;
        let count = self.state.storage.children(allocation).len();
        match (operation, values) {
            (
                CollectionBuiltinOp::PushGrowable | CollectionBuiltinOp::PushFixed,
                [Some(value), None],
            ) => {
                if operation == CollectionBuiltinOp::PushFixed {
                    let Some(metadata) = self.constants.metadata(collection.shape) else {
                        return Err(unsupported_error(
                            labels::FIXED_COLLECTION_PUSH,
                            span,
                            self.string_table,
                        ));
                    };
                    let Some(capacity) = self.types.collection_fixed_capacity(metadata.type_id)
                    else {
                        return Err(unsupported_error(
                            labels::FIXED_COLLECTION_PUSH,
                            span,
                            self.string_table,
                        ));
                    };
                    if count >= capacity {
                        if self.state.active_handler.body.is_none() {
                            return Err(unsupported_error(
                                labels::FIXED_COLLECTION_PUSH,
                                span,
                                self.string_table,
                            ));
                        }
                        self.execute_collection_handler(
                            BuiltinErrorCode::CollectionFixedCapacityExceeded,
                            bindings,
                        )?;
                        return Ok(true);
                    }
                }
                if !self.state.storage.push(allocation, value, self.budget)? {
                    return Err(unsupported_error(
                        labels::COLLECTION_MUTATION,
                        span,
                        self.string_table,
                    ));
                }
            }
            (CollectionBuiltinOp::Set, [Some(index_value), Some(value)]) => {
                let index = constant_index(self.constants, index_value.shape);
                if collection_index_is_out_of_bounds(
                    self.constants,
                    index,
                    index_value.shape,
                    count,
                ) {
                    self.handle_collection_bounds_error(span, bindings)?;
                    return Ok(true);
                }
                let Some(index) = index else {
                    return Err(unsupported_error(
                        labels::COLLECTION_MUTATION,
                        span,
                        self.string_table,
                    ));
                };
                let Some(place) = self.state.storage.project(collection, index)? else {
                    return Err(unsupported_error(
                        labels::COLLECTION_MUTATION,
                        span,
                        self.string_table,
                    ));
                };
                if !self.state.storage.replace(place, value, self.budget)? {
                    return Err(unsupported_error(
                        labels::COLLECTION_MUTATION,
                        span,
                        self.string_table,
                    ));
                }
            }
            (CollectionBuiltinOp::Remove, [Some(index_value), None]) => {
                let index = constant_index(self.constants, index_value.shape);
                if collection_index_is_out_of_bounds(
                    self.constants,
                    index,
                    index_value.shape,
                    count,
                ) {
                    self.handle_collection_bounds_error(span, bindings)?;
                    return Ok(true);
                }
                let Some(index) = index else {
                    return Err(unsupported_error(
                        labels::COLLECTION_REMOVAL,
                        span,
                        self.string_table,
                    ));
                };
                self.state.storage.remove(allocation, index, self.budget)?;
            }
            _ => return Err(collection_mutation_arity()),
        }
        Ok(true)
    }

    fn handle_collection_bounds_error(
        &mut self,
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
                    let Some(index) = constant_index(self.constants, value.shape) else {
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

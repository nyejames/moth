//! Checked semantic constructors for the phase expression owner.
//!
//! Constructors consume canonical resolved types and facts. Constant content belongs to the
//! separate constant store; expression rows retain occurrence metadata and immutable ID edges.

use super::call_argument::CallArgument;
use super::expression_kind::Operator;
use super::expression_rpn::{ResolvedOperation, ResolvedOperationKind};
use super::expression_store::*;
pub use super::expression_types::{
    ConstRecordState, ConstValueKind, FallibleExpressionHandling, FallibleHandling,
};
use super::failure_facts::{
    ExpressionFailureFacts, ImplicitFailureContributor, ImplicitFailureSource, TypedErrorProducer,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueId, ConstValueMetadata, ConstValuePayload, ConstValueStore,
    ConstValueStoreError,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{Template, TemplateConstValueKind};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CompileTimeEvaluationErrorReason, CompilerDiagnostic,
    InvalidBuiltinCallReason, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

pub(crate) struct CollectionExpressionType {
    pub(crate) element_type_id: TypeId,
    pub(crate) fixed_capacity: Option<usize>,
    pub(crate) collection_type_id: Option<TypeId>,
}

pub(crate) struct MapLiteralExpressionType {
    pub(crate) key_type_id: TypeId,
    pub(crate) value_type_id: TypeId,
    pub(crate) map_type_id: Option<TypeId>,
}

pub(crate) struct HandledFallibleHostFunctionCallInput<A> {
    pub(crate) id: ExternalFunctionId,
    pub(crate) args: A,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) requires_external_float_validation: bool,
    pub(crate) error_type_id: TypeId,
    pub(crate) handling: FallibleExpressionHandling,
    pub(crate) span: Option<SourceSpan>,
}

pub(crate) struct ChoiceConstructInput<F> {
    pub(crate) tag: usize,
    pub(crate) fields: F,
    pub(crate) type_id: TypeId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) value_mode: ValueMode,
}

impl From<ConstValueStoreError> for ExpressionConstructionFailure {
    fn from(error: ConstValueStoreError) -> Self {
        match error {
            ConstValueStoreError::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ConstValueStoreError::Infrastructure(error) => Self::Invariant(error),
        }
    }
}

impl AstExpressionBuilder {
    pub(crate) fn field_access(
        &mut self,
        base: OwnedExpr,
        field: StringId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        value_mode: ValueMode,
        const_record_state: ConstRecordState,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let mut input = self.completed_input(
            ExpressionOp::FieldAccess { base, field },
            SuccessShape::One(type_id),
            span,
            value_mode,
        )?;
        input.classification.const_record_state = const_record_state;
        self.append_completed(input)
    }

    pub(crate) fn append_expression(
        &mut self,
        operation: ExpressionOp<OwnedExpr>,
        span: Option<SourceSpan>,
        type_id: TypeId,
        value_mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.append_with_success_shape(operation, span, SuccessShape::One(type_id), value_mode)
    }

    pub(crate) fn append_with_success_shape(
        &mut self,
        operation: ExpressionOp<OwnedExpr>,
        span: Option<SourceSpan>,
        success: SuccessShape,
        value_mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let input = self.completed_input(operation, success, span, value_mode)?;
        self.append_completed(input)
    }

    fn snapshot(
        &self,
        id: OwnedExpr,
    ) -> Result<CompletedExpressionInput, ExpressionConstructionFailure> {
        Ok(self.view_owned(id)?.to_completed_input(self)?)
    }

    pub(crate) fn completed_input(
        &self,
        operation: ExpressionOp<OwnedExpr>,
        success: SuccessShape,
        span: Option<SourceSpan>,
        value_mode: ValueMode,
    ) -> Result<CompletedExpressionInput, ExpressionConstructionFailure> {
        let raw_operation = self.check_operation_owned(operation)?;
        let failure =
            ExpressionFailureFacts::from_operation(self.operation_view(raw_operation)?, span);
        let mut classification = ExpressionClassification {
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
        };
        let mut provenance = SyntheticInterfaceProvenance::empty();
        self.visit_operation_children(raw_operation, |child| {
            classification.contains_regular_division |=
                child.classification().contains_regular_division;
            if let Some(occurrence) = child.occurrence_facts() {
                provenance = provenance.union(&occurrence.provenance);
            }
        })?;
        let occurrence = (!provenance.is_empty()).then_some(OccurrenceFacts {
            provenance,
            diagnostic_spelling: None,
        });
        Ok(CompletedExpressionInput {
            operation,
            success,
            error_type: None,
            span,
            value_mode,
            classification,
            failure: (failure != ExpressionFailureFacts::default()).then_some(failure),
            occurrence,
        })
    }

    pub(crate) fn success_from_slots(
        &mut self,
        slots: Vec<TypeId>,
        span: Option<SourceSpan>,
    ) -> Result<SuccessShape, ExpressionConstructionFailure> {
        match slots.as_slice() {
            [] => Ok(SuccessShape::Zero),
            [single] => Ok(SuccessShape::One(*single)),
            _ => Ok(SuccessShape::Multiple(self.append_types(slots, span)?)),
        }
    }

    pub(crate) fn constant(
        &mut self,
        value: ConstValueId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        value_mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.append_expression(ExpressionOp::Constant(value), span, type_id, value_mode)
    }

    fn literal(
        &mut self,
        values: &mut ConstValueStore,
        payload: ConstValuePayload,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let value =
            values.insert_value(ConstValueMetadata::resolved(type_id, span, mode), payload)?;
        self.constant(value, type_id, span, mode)
    }

    pub(crate) fn number(
        &mut self,
        values: &mut ConstValueStore,
        value: NumberValue,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.literal(
            values,
            ConstValuePayload::Number(value),
            type_id,
            span,
            mode,
        )
    }

    pub(crate) fn fixed_scalar(
        &mut self,
        values: &mut ConstValueStore,
        value: FixedScalarValue,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.literal(
            values,
            ConstValuePayload::FixedScalar(value),
            builtin_type_ids::fixed_scalar(value.scalar()),
            span,
            mode,
        )
    }

    pub(crate) fn float_from_external_constant(
        &mut self,
        values: &mut ConstValueStore,
        value: f64,
        name: StringId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let materialised =
            FixedScalarValue::binary_float(FixedScalar::F64, value).ok_or_else(|| {
                ExpressionConstructionFailure::Diagnostic(
                    CompilerDiagnostic::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::FloatOverflow,
                        Some(name),
                        span,
                    ),
                )
            })?;
        self.fixed_scalar(values, materialised, span, mode)
    }

    pub(crate) fn string_slice(
        &mut self,
        values: &mut ConstValueStore,
        value: StringId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.literal(
            values,
            ConstValuePayload::String(ConstStringValue::Text(value)),
            builtin_type_ids::STRING,
            span,
            mode,
        )
    }

    pub(crate) fn bool(
        &mut self,
        values: &mut ConstValueStore,
        value: bool,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.literal(
            values,
            ConstValuePayload::Bool(value),
            builtin_type_ids::BOOL,
            span,
            mode,
        )
    }

    pub(crate) fn char(
        &mut self,
        values: &mut ConstValueStore,
        value: char,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.literal(
            values,
            ConstValuePayload::Char(value),
            builtin_type_ids::CHAR,
            span,
            mode,
        )
    }

    pub(crate) fn reference_with_type_id(
        &mut self,
        path: PathId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
        state: ConstRecordState,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let mut input = self.completed_input(
            ExpressionOp::Reference(path),
            SuccessShape::One(type_id),
            span,
            mode,
        )?;
        input.classification.const_record_state = state;
        self.append_completed(input)
    }

    pub(crate) fn copy_with_type_id(
        &mut self,
        place: PlaceId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.append_expression(ExpressionOp::Copy(place), span, type_id, mode.as_owned())
    }

    pub(crate) fn option_none_with_type_id(
        &mut self,
        values: &mut ConstValueStore,
        inner: TypeId,
        environment: &mut TypeEnvironment,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let type_id = environment.intern_option(inner);
        self.literal(
            values,
            ConstValuePayload::OptionNone,
            type_id,
            span,
            ValueMode::ImmutableOwned,
        )
    }

    fn preflight_call_inputs(
        &self,
        receiver: Option<OwnedExpr>,
        args: &ExpressionInputs<'_, CallArgument, CallArgument<OwnedExpr>>,
    ) -> Result<(), ExpressionConstructionFailure> {
        // Foreign owners take precedence over invalid indices anywhere in the call inputs.
        if let Some(receiver) = receiver {
            self.check_owner(receiver)?;
        }
        match args {
            ExpressionInputs::Attached(args) => {
                self.check_owned_slice(*args)?;
            }
            ExpressionInputs::Owned(args) => {
                for argument in args {
                    self.check_owner(argument.value)?;
                }
            }
        }
        if let Some(receiver) = receiver {
            self.view(receiver.id())?;
        }
        Ok(())
    }

    fn check_argument_parameter_slot<E>(
        argument: &CallArgument<E>,
    ) -> Result<(), ExpressionConstructionFailure> {
        if let Some(slot) = argument.parameter_slot {
            u32::try_from(slot.index()).map_err(|_| {
                ExpressionConstructionFailure::Diagnostic(
                    CompilerDiagnostic::compiler_capacity_exceeded(
                        AstCapacityResource::ExpressionSideEntries,
                        argument.span,
                    ),
                )
            })?;
        }
        Ok(())
    }

    fn resolved_argument<E>(argument: &CallArgument<E>, value: ExprId) -> ResolvedArgument {
        // The whole argument list has passed parameter-slot checks before installation.
        let parameter_slot = argument.parameter_slot.map(|slot| slot.index() as u32);
        ResolvedArgument {
            value,
            target_param: argument.target_param,
            parameter_slot,
            access_mode: argument.access_mode,
            passing_mode: argument.passing_mode,
            span: argument.span,
            target_span: argument.target_span,
            marker_span: argument.marker_span,
            following_separator_span: argument.following_separator_span,
        }
    }

    fn resolved_arguments(
        &mut self,
        args: ExpressionInputs<'_, CallArgument, CallArgument<OwnedExpr>>,
        span: Option<SourceSpan>,
    ) -> Result<ArgumentRange, ExpressionConstructionFailure> {
        // All callers preflight owners and the receiver before validating argument indices.
        let (start, len) = match args {
            ExpressionInputs::Attached(args) => {
                let args = self.check_owned_slice(args)?;
                for argument in args {
                    self.view(argument.value)?;
                    Self::check_argument_parameter_slot(argument)?;
                }
                let extent = checked_extent(
                    self.storage.arguments.len(),
                    args.len(),
                    self.limit,
                    span,
                    false,
                )?;
                self.storage.arguments.extend(
                    args.iter()
                        .map(|argument| Self::resolved_argument(argument, argument.value)),
                );
                extent
            }
            ExpressionInputs::Owned(args) => {
                for argument in &args {
                    self.view(argument.value.id())?;
                    Self::check_argument_parameter_slot(argument)?;
                }
                let extent = checked_extent(
                    self.storage.arguments.len(),
                    args.len(),
                    self.limit,
                    span,
                    false,
                )?;
                self.storage.arguments.extend(
                    args.into_iter()
                        .map(|argument| Self::resolved_argument(&argument, argument.value.id())),
                );
                extent
            }
        };
        Ok(ArgumentRange { start, len })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "call assembly keeps the mutable builder, resolved callable, receiver, owned arguments, slots, handling record, external-validation flag and span as separate inputs"
    )]
    fn call_with_target<'a>(
        &mut self,
        target: ResolvedCallableTarget,
        receiver: Option<OwnedExpr>,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        handling: Option<HandlingRecord>,
        external_validation: bool,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let args = args.into();
        self.preflight_call_inputs(receiver, &args)?;
        let arguments = self.resolved_arguments(args, span)?;
        let success = self.success_from_slots(slots, span)?;
        let error_type = handling.as_ref().and_then(|record| record.error_type);
        let handling = handling
            .map(|record| self.append_handling(record, span))
            .transpose()?;
        let call = self.append_call(
            ResolvedCall {
                target,
                arguments,
                handling,
                requires_external_float_validation: external_validation,
                error_type,
                span,
            },
            span,
        )?;
        let operation = match receiver {
            Some(receiver) => ExpressionOp::MethodCall { receiver, call },
            None => ExpressionOp::Call(call),
        };
        self.append_with_success_shape(operation, span, success, ValueMode::MutableOwned)
    }

    pub(crate) fn function_call_with_typed_arguments<'a>(
        &mut self,
        name: PathId,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.call_with_target(
            ResolvedCallableTarget::Function(name),
            None,
            args,
            slots,
            None,
            false,
            span,
        )
    }

    pub(crate) fn method_call_with_typed_arguments<'a>(
        &mut self,
        receiver: OwnedExpr,
        name: PathId,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.call_with_target(
            ResolvedCallableTarget::Function(name),
            Some(receiver),
            args,
            slots,
            None,
            false,
            span,
        )
    }

    pub(crate) fn handled_fallible_function_call_with_typed_arguments<'a>(
        &mut self,
        name: PathId,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        error_type: TypeId,
        handling: FallibleExpressionHandling,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.call_with_target(
            ResolvedCallableTarget::Function(name),
            None,
            args,
            slots,
            Some(HandlingRecord {
                handling,
                propagation_span: None,
                error_type: Some(error_type),
            }),
            false,
            span,
        )
    }

    pub(crate) fn host_function_call_with_typed_arguments<'a>(
        &mut self,
        id: ExternalFunctionId,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        validation: bool,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.call_with_target(
            ResolvedCallableTarget::Host(id),
            None,
            args,
            slots,
            None,
            validation,
            span,
        )
    }

    pub(crate) fn handled_fallible_host_function_call_with_typed_arguments<'a>(
        &mut self,
        input: HandledFallibleHostFunctionCallInput<
            impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        >,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.call_with_target(
            ResolvedCallableTarget::Host(input.id),
            None,
            input.args,
            input.result_type_ids,
            Some(HandlingRecord {
                handling: input.handling,
                propagation_span: None,
                error_type: Some(input.error_type_id),
            }),
            input.requires_external_float_validation,
            input.span,
        )
    }

    pub(crate) fn collection_builtin_call_with_typed_arguments<'a>(
        &mut self,
        receiver: OwnedExpr,
        operation: CollectionBuiltinOp,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let args = args.into();
        self.preflight_call_inputs(Some(receiver), &args)?;
        let arguments = self.resolved_arguments(args, span)?;
        let success = self.success_from_slots(slots, span)?;
        let call = self.append_collection_call(
            CollectionCall {
                operation,
                arguments,
                span,
            },
            span,
        )?;
        self.append_with_success_shape(
            ExpressionOp::CollectionBuiltin { receiver, call },
            span,
            success,
            ValueMode::MutableOwned,
        )
    }

    pub(crate) fn map_builtin_call_with_typed_arguments<'a>(
        &mut self,
        receiver: OwnedExpr,
        operation: MapBuiltinOp,
        receiver_requires_mutable: bool,
        args: impl Into<ExpressionInputs<'a, CallArgument, CallArgument<OwnedExpr>>>,
        slots: Vec<TypeId>,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let args = args.into();
        self.preflight_call_inputs(Some(receiver), &args)?;
        let arguments = self.resolved_arguments(args, span)?;
        let success = self.success_from_slots(slots, span)?;
        let call = self.append_map_call(
            MapCall {
                operation,
                receiver_requires_mutable,
                arguments,
                span,
            },
            span,
        )?;
        self.append_with_success_shape(
            ExpressionOp::MapBuiltin { receiver, call },
            span,
            success,
            ValueMode::MutableOwned,
        )
    }

    pub(crate) fn cast(
        &mut self,
        source: OwnedExpr,
        facts: CastFacts,
        result: TypeId,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let span = facts.span;
        let mode = self.view_owned(source)?.value_mode();
        let facts = self.append_cast(facts, span)?;
        self.append_expression(ExpressionOp::Cast { source, facts }, span, result, mode)
    }

    pub(crate) fn coerced(
        &mut self,
        value: OwnedExpr,
        to_type: TypeId,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let child = self.view_owned(value)?;
        let span = child.span();
        let mode = child.value_mode();
        let state = child.classification().const_record_state;
        let mut input = self.completed_input(
            ExpressionOp::Coerce {
                value,
                operation: CoercionOperation::ToType(to_type),
            },
            SuccessShape::One(to_type),
            span,
            mode,
        )?;
        input.classification.const_record_state = state;
        self.append_completed(input)
    }

    pub(crate) fn handled_result(
        &mut self,
        value: OwnedExpr,
        handling: FallibleExpressionHandling,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let child = self.view_owned(value)?;
        let success = child.success_shape();
        let error_type = child.error_type();
        let handling = self.append_handling(
            HandlingRecord {
                handling,
                propagation_span: None,
                error_type,
            },
            span,
        )?;
        self.append_with_success_shape(
            ExpressionOp::HandleFallible { value, handling },
            span,
            success,
            ValueMode::ImmutableOwned,
        )
    }

    pub(crate) fn option_propagation_with_type_id(
        &mut self,
        value: OwnedExpr,
        inner: TypeId,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.append_expression(
            ExpressionOp::PropagateOption { value },
            span,
            inner,
            ValueMode::ImmutableOwned,
        )
    }

    pub(crate) fn collection_with_type_id<'a>(
        &mut self,
        items: impl Into<ExpressionInputs<'a, ExprId, OwnedExpr>>,
        collection: CollectionExpressionType,
        environment: &mut TypeEnvironment,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let items = self.append_children(items, span)?;
        let type_id = collection.collection_type_id.unwrap_or_else(|| {
            environment.intern_collection(collection.element_type_id, collection.fixed_capacity)
        });
        self.append_expression(ExpressionOp::Collection(items), span, type_id, mode)
    }

    pub(crate) fn map_literal_with_type_id<'a>(
        &mut self,
        entries: impl Into<ExpressionInputs<'a, MapEntry, MapEntry<OwnedExpr>>>,
        map: MapLiteralExpressionType,
        environment: &mut TypeEnvironment,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let entries = self.append_map_entries(entries, span)?;
        let type_id = map
            .map_type_id
            .unwrap_or_else(|| environment.intern_map(map.key_type_id, map.value_type_id));
        self.append_expression(ExpressionOp::Map(entries), span, type_id, mode)
    }

    pub(crate) fn struct_instance<'a>(
        &mut self,
        fields: impl Into<ExpressionInputs<'a, ConstructorField, ConstructorField<OwnedExpr>>>,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
        const_record: bool,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let fields = self.append_constructor_fields(fields, span)?;
        let mut input = self.completed_input(
            ExpressionOp::Struct { fields },
            SuccessShape::One(type_id),
            span,
            mode,
        )?;
        if const_record {
            input.classification.const_record_state = ConstRecordState::ConstRecord;
        }
        self.append_completed(input)
    }

    pub(crate) fn choice_construct<'a>(
        &mut self,
        input: ChoiceConstructInput<
            impl Into<ExpressionInputs<'a, ConstructorField, ConstructorField<OwnedExpr>>>,
        >,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let tag = u32::try_from(input.tag).map_err(|_| {
            ExpressionConstructionFailure::Diagnostic(
                CompilerDiagnostic::compiler_capacity_exceeded(
                    AstCapacityResource::ChoiceTags,
                    input.span,
                ),
            )
        })?;
        let fields = self.append_constructor_fields(input.fields, input.span)?;
        self.append_expression(
            ExpressionOp::Choice { tag, fields },
            input.span,
            input.type_id,
            input.value_mode,
        )
    }

    pub(crate) fn value_block(
        &mut self,
        block: ValueBlock,
        span: Option<SourceSpan>,
        success: SuccessShape,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let block = self.append_value_block(block, span)?;
        self.append_with_success_shape(ExpressionOp::ValueBlock(block), span, success, mode)
    }

    pub(crate) fn template(
        &mut self,
        template: Template,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let span = template.span;
        let template = self.append_template(template, span)?;
        self.append_expression(
            ExpressionOp::Template(template),
            span,
            builtin_type_ids::STRING,
            mode,
        )
    }

    pub(crate) fn resolved_operation(
        &mut self,
        left: OwnedExpr,
        right: Option<OwnedExpr>,
        facts: ResolvedOperation,
        mode: ValueMode,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.check_owner(left)?;
        if let Some(right) = right {
            self.check_owner(right)?;
        }
        if facts.operator.required_values() != 1 + usize::from(right.is_some()) {
            return Err(invariant("resolved operation arity mismatch").into());
        }
        if self.view_owned(left)?.value_type_id()? != facts.left_type {
            return Err(invariant("resolved left operand type mismatch").into());
        }
        if right
            .map(|id| self.view_owned(id).and_then(|view| view.value_type_id()))
            .transpose()?
            != facts.right_type
        {
            return Err(invariant("resolved right operand type mismatch").into());
        }
        let span = facts.span;
        let result = facts.result_type;
        let operation = match facts.kind {
            ResolvedOperationKind::Numeric(_) => {
                let operation = self.append_numeric_operation(facts, span)?;
                match right {
                    Some(right) => ExpressionOp::NumericBinary {
                        left,
                        right,
                        facts: operation,
                    },
                    None => ExpressionOp::NumericUnary {
                        operand: left,
                        facts: operation,
                    },
                }
            }
            ResolvedOperationKind::Comparison(_) => ExpressionOp::Compare {
                left,
                right: right.ok_or_else(|| invariant("comparison requires two operands"))?,
                facts: self.append_comparison_operation(facts, span)?,
            },
            ResolvedOperationKind::Range { .. } => ExpressionOp::Range {
                start: left,
                end: right.ok_or_else(|| invariant("range requires two operands"))?,
                facts: self.append_range_operation(facts, span)?,
            },
            ResolvedOperationKind::NonNumeric => match facts.operator {
                Operator::Not => ExpressionOp::LogicalNot { operand: left },
                Operator::And | Operator::Or => ExpressionOp::LogicalBinary {
                    left,
                    right: right
                        .ok_or_else(|| invariant("logical operation requires two operands"))?,
                    operation: if facts.operator == Operator::And {
                        LogicalOperator::And
                    } else {
                        LogicalOperator::Or
                    },
                },
                Operator::Equality
                | Operator::NotEqual
                | Operator::LessThan
                | Operator::LessThanOrEqual
                | Operator::GreaterThan
                | Operator::GreaterThanOrEqual => ExpressionOp::Compare {
                    left,
                    right: right.ok_or_else(|| invariant("comparison requires two operands"))?,
                    facts: self.append_comparison_operation(facts, span)?,
                },
                _ => return Err(invariant("non-numeric operation has numeric operator").into()),
            },
        };
        let mut input = self.completed_input(operation, SuccessShape::One(result), span, mode)?;
        input.classification.contains_regular_division |= facts.operator == Operator::Divide;
        self.append_completed(input)
    }

    pub(crate) fn commit_replacement(
        &mut self,
        input: CompletedExpressionInput,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.construct(|candidate| {
            let root = candidate.append_replacement(input)?;
            Ok(CandidateEdge::New(root))
        })
    }

    pub(crate) fn with_failure_facts(
        &mut self,
        id: OwnedExpr,
        facts: ExpressionFailureFacts,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        if self.view_owned(id)?.failure_facts() == Some(&facts) {
            return Ok(id);
        }
        let mut input = self.snapshot(id)?;
        input.failure = Some(facts);
        self.commit_replacement(input)
    }

    pub(crate) fn with_typed_error_producer(
        &mut self,
        id: OwnedExpr,
        error: TypeId,
        builtin_name: Option<StringId>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let mut input = self.snapshot(id)?;
        let operation = self.view_owned(id)?.operation();
        let handled = match operation {
            ExpressionOperationView::HandleFallible { .. } => true,
            ExpressionOperationView::Call(call)
            | ExpressionOperationView::MethodCall { call, .. } => call.handling().is_some(),
            _ => false,
        };
        if let ExpressionOperationView::Call(call)
        | ExpressionOperationView::MethodCall { call, .. } = operation
        {
            let metadata = call.metadata();
            let mut record = ResolvedCall {
                target: metadata.target,
                arguments: metadata.arguments,
                handling: metadata.handling,
                requires_external_float_validation: metadata.requires_external_float_validation,
                error_type: Some(error),
                span: metadata.span,
            };
            let handling = call.handling().map(|handling| HandlingRecord {
                handling: handling.handling,
                error_type: Some(error),
                propagation_span: handling.propagation_span,
            });
            if let Some(handling) = handling {
                record.handling = Some(self.append_handling(handling, input.span)?);
            }
            let call = self.append_call(record, input.span)?;
            input.operation = match input.operation {
                ExpressionOp::MethodCall { receiver, .. } => {
                    ExpressionOp::MethodCall { receiver, call }
                }
                _ => ExpressionOp::Call(call),
            };
        }
        if !handled {
            input.error_type = Some(error);
        }
        input
            .failure
            .get_or_insert_with(ExpressionFailureFacts::default)
            .record_typed_error(TypedErrorProducer {
                span: input.span,
                error_type_id: error,
                builtin_name,
            });
        self.commit_replacement(input)
    }

    pub(crate) fn with_private_call_failure_candidate(
        &mut self,
        id: OwnedExpr,
        path: PathId,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let mut input = self.snapshot(id)?;
        input
            .failure
            .get_or_insert_with(ExpressionFailureFacts::default)
            .record_implicit(ImplicitFailureContributor {
                span: input.span,
                codes: &[],
                source: ImplicitFailureSource::PrivateCall(path),
            });
        self.commit_replacement(input)
    }

    #[cfg(test)]
    pub(crate) fn with_regular_division_provenance(
        &mut self,
        id: OwnedExpr,
        contains: bool,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        if self
            .view_owned(id)?
            .classification()
            .contains_regular_division
            == contains
        {
            return Ok(id);
        }
        let mut input = self.snapshot(id)?;
        input.classification.contains_regular_division = contains;
        self.commit_replacement(input)
    }

    pub(crate) fn with_synthetic_interface_provenance(
        &mut self,
        id: OwnedExpr,
        provenance: SyntheticInterfaceProvenance,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let current = self.view_owned(id)?.occurrence_facts();
        if current.map_or(provenance.is_empty(), |current| {
            current.provenance == provenance
        }) {
            return Ok(id);
        }
        let mut input = self.snapshot(id)?;
        let diagnostic_spelling = input
            .occurrence
            .as_ref()
            .and_then(|current| current.diagnostic_spelling);
        input.occurrence = Some(OccurrenceFacts {
            provenance,
            diagnostic_spelling,
        });
        self.commit_replacement(input)
    }

    /// Replacement occurrence metadata comes from the authored use, not constant content.
    pub(crate) fn preserve_authored_metadata(
        &mut self,
        replacement: OwnedExpr,
        authored: OwnedExpr,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        self.check_owner(replacement)?;
        self.check_owner(authored)?;
        self.check_owned(replacement)?;
        self.check_owned(authored)?;
        if replacement == authored {
            return Ok(replacement);
        }
        let mut input = self.snapshot(replacement)?;
        let authored = self.view_owned(authored)?;
        input.span = authored.span();
        input.value_mode = authored.value_mode();
        input.classification.const_record_state = authored.classification().const_record_state;
        input.classification.contains_regular_division |=
            authored.classification().contains_regular_division;
        input.failure = authored.failure_facts().cloned();
        if let Some(occurrence) = authored.occurrence_facts() {
            let existing = input.occurrence.take();
            let provenance = existing.as_ref().map_or_else(
                || occurrence.provenance.clone(),
                |existing| existing.provenance.union(&occurrence.provenance),
            );
            input.occurrence = Some(OccurrenceFacts {
                provenance,
                diagnostic_spelling: occurrence.diagnostic_spelling,
            });
        }
        self.commit_replacement(input)
    }

    /// Postfix source evidence gets a new handling record and root so shared occurrences stay
    /// immutable even when they use the same resolved call.
    pub(crate) fn with_propagation_span(
        &mut self,
        id: OwnedExpr,
        span: Option<SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let mut input = self.snapshot(id)?;
        let handling = match self.view_owned(id)?.operation() {
            ExpressionOperationView::HandleFallible { handling, .. } => handling,
            ExpressionOperationView::Call(call)
            | ExpressionOperationView::MethodCall { call, .. } => call
                .handling()
                .ok_or_else(|| invariant("postfix span requires handled call"))?,
            _ => return Err(invariant("postfix span requires handled expression").into()),
        };
        if handling.handling != FallibleExpressionHandling::Propagate {
            return Err(invariant("postfix span requires propagation handling").into());
        }
        if handling.propagation_span == span {
            return Ok(id);
        }
        let handling = HandlingRecord {
            handling: handling.handling,
            error_type: handling.error_type,
            propagation_span: span,
        };
        let handling = self.append_handling(handling, input.span)?;
        input.operation = match input.operation {
            ExpressionOp::HandleFallible { value, .. } => {
                ExpressionOp::HandleFallible { value, handling }
            }
            ExpressionOp::Call(_) | ExpressionOp::MethodCall { .. } => {
                let view = self.view_owned(id)?;
                let (ExpressionOperationView::Call(call)
                | ExpressionOperationView::MethodCall { call, .. }) = view.operation()
                else {
                    unreachable!("operation matched above");
                };
                let metadata = call.metadata();
                let record = ResolvedCall {
                    target: metadata.target,
                    arguments: metadata.arguments,
                    handling: Some(handling),
                    requires_external_float_validation: metadata.requires_external_float_validation,
                    error_type: metadata.error_type,
                    span: metadata.span,
                };
                let call = self.append_call(record, input.span)?;
                match input.operation {
                    ExpressionOp::MethodCall { receiver, .. } => {
                        ExpressionOp::MethodCall { receiver, call }
                    }
                    _ => ExpressionOp::Call(call),
                }
            }
            _ => unreachable!("operation matched above"),
        };
        let facts = input
            .failure
            .get_or_insert_with(ExpressionFailureFacts::default);
        facts.postfix_exit_span = span.or(facts.postfix_exit_span);
        self.commit_replacement(input)
    }
}

impl ExpressionView<'_> {
    pub(crate) fn is_const_record_value(self) -> bool {
        self.classification().const_record_state == ConstRecordState::ConstRecord
    }

    pub(crate) fn propagation_span(self) -> Option<SourceSpan> {
        match self.operation() {
            ExpressionOperationView::HandleFallible { handling, .. } => handling.propagation_span,
            ExpressionOperationView::Call(call)
            | ExpressionOperationView::MethodCall { call, .. } => call
                .handling()
                .and_then(|handling| handling.propagation_span),
            _ => None,
        }
    }

    pub(crate) fn unhandled_typed_error_diagnostic(self) -> Option<CompilerDiagnostic> {
        let facts = self.failure_facts()?;
        if facts.disposition != super::failure_facts::FailureDisposition::Pending {
            return None;
        }
        facts.summary.first_typed.map(|producer| {
            let span = producer.span.or(self.span());
            match producer.builtin_name {
                Some(name) => CompilerDiagnostic::invalid_builtin_call(
                    InvalidBuiltinCallReason::UnhandledFallibleCall,
                    Some(name),
                    span,
                ),
                None => CompilerDiagnostic::invalid_fallible_handling(
                    InvalidFallibleHandlingReason::UnhandledErrorReturn,
                    span,
                ),
            }
        })
    }

    pub(crate) fn const_value_kind_with_template_classifier(
        self,
        constants: &ConstValueStore,
        classify: &mut impl FnMut(&Template) -> Result<TemplateConstValueKind, TemplateError>,
    ) -> Result<ConstValueKind, TemplateError> {
        use ExpressionOperationView as V;
        let kind = match self.operation() {
            V::Constant(id) => {
                constants
                    .metadata(id)
                    .ok_or_else(|| invariant("constant expression outside content owner"))?
                    .value_kind
            }
            V::Collection(values) => {
                for value in values.iter() {
                    if value.is_const_record_value()
                        || !value
                            .const_value_kind_with_template_classifier(constants, classify)?
                            .is_compile_time_value()
                    {
                        return Ok(ConstValueKind::NonConst);
                    }
                }
                ConstValueKind::Composite
            }
            V::Struct { fields } | V::Choice { fields, .. } => {
                for field in fields.iter() {
                    if !field
                        .value()
                        .const_value_kind_with_template_classifier(constants, classify)?
                        .is_compile_time_value()
                    {
                        return Ok(ConstValueKind::NonConst);
                    }
                }
                ConstValueKind::Composite
            }
            V::Range { start, end, .. }
                if start
                    .const_value_kind_with_template_classifier(constants, classify)?
                    .is_compile_time_value()
                    && end
                        .const_value_kind_with_template_classifier(constants, classify)?
                        .is_compile_time_value() =>
            {
                ConstValueKind::Composite
            }
            V::Template(template) => match classify(template)? {
                TemplateConstValueKind::RenderableString => ConstValueKind::RenderableTemplate,
                TemplateConstValueKind::WrapperTemplate => ConstValueKind::TemplateWrapper,
                TemplateConstValueKind::SlotInsertHelper => ConstValueKind::SlotInsertTemplate,
                TemplateConstValueKind::NonConst => ConstValueKind::NonConst,
            },
            V::Reference(_) if self.is_const_record_value() => ConstValueKind::Composite,
            V::Coerce { value, .. } => {
                value.const_value_kind_with_template_classifier(constants, classify)?
            }
            _ => ConstValueKind::NonConst,
        };
        Ok(kind)
    }
}

#[cfg(test)]
#[path = "tests/external_float_constant_tests.rs"]
mod external_float_constant_tests;

#[cfg(test)]
#[path = "tests/success_shape_tests.rs"]
mod success_shape_tests;

#[cfg(test)]
#[path = "tests/runtime_handoff_expression_payload_tests.rs"]
mod runtime_handoff_expression_payload_tests;

//! Retained generic struct and choice constructors.
//!
//! WHAT: suspends a generic constructor whose arguments still hold pending numeric origins as one
//! deferred call. Its result shape is linked at parse time from the declared fields and the
//! argument origins, so the consumer sees the nominal's declared self shape. Completion resolves
//! the arguments, then runs the same generic inference and finisher as the immediate lane.
//! WHY: a generic field's numeric type is known only from later evidence. Defaulting a literal to
//! I32 during argument parsing would reject values that the generic instance can represent.

use crate::compiler_frontend::ast::expressions::call_validation::CallDiagnosticName;
use crate::compiler_frontend::ast::expressions::choice_constructor::{
    GenericChoiceCompletion, complete_generic_choice_instance,
};
use crate::compiler_frontend::ast::expressions::constructor_views::ConstructorField;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::OwnedExpr;
use crate::compiler_frontend::ast::expressions::generic_nominal_inference::GenericNominalConstructorSeed;
use crate::compiler_frontend::ast::expressions::struct_instance::{
    GenericStructCompletion, complete_generic_struct_instance,
};
use crate::compiler_frontend::ast::generic_functions::choose_retained_argument_destination;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredCallArgument, DeferredExpressionCompletion, DeferredExpressionServices,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeId, materialise_known_shape,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// The constructor-specific facts a retained generic nominal call finishes with.
///
/// WHAT: structs retain declared field views; choices retain their selected variant and payload
///       views with the construction tag and const policy.
/// WHY: body-root completion consumes the same per-call views the parser used rather than building
///      a second view list from the nominal declaration.
#[derive(Debug, Clone)]
pub(crate) enum GenericNominalConstructor {
    Struct {
        fields: Vec<ConstructorField>,
        struct_value_mode: ValueMode,
        enforce_const_record: bool,
    },
    Choice {
        variant: ChoiceVariantDefinition,
        payload_field_views: Vec<ConstructorField>,
        variant_index: usize,
        variant_span: Option<SourceSpan>,
        enforce_constant_context: bool,
        route_future_input: Option<bool>,
    },
}

/// One suspended generic struct or choice constructor.
///
/// WHAT: the nominal identity and its contextual seed, the ordered retained arguments, the symbolic
///       result shape the consumer receives, and the constructor facts used at completion.
/// WHY: the instance is inferred only from completed argument types, so no pending instance key is
///      interned early. The result shape is the nominal's declared self shape, which is the same
///      identity the declaration site uses.
#[derive(Debug, Clone)]
pub(crate) struct DeferredGenericNominalCall {
    pub(crate) nominal_path: PathId,
    pub(crate) diagnostic_name: CallDiagnosticName<'static>,
    pub(crate) seed: GenericNominalConstructorSeed,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    pub(crate) result_shape: KnownShapeId,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) constructor: GenericNominalConstructor,
}

impl DeferredGenericNominalCall {
    /// Wrap this suspended call as the one pending operand its expression position carries.
    pub(crate) fn into_pending_operand(self) -> ExpressionRpnItem {
        let span = self.call_span;
        ExpressionRpnItem::PendingNumericCall {
            call: Box::new(DeferredCall::GenericNominal(Box::new(self))),
            span,
        }
    }
}

/// Resume one retained generic nominal constructor once its numeric origins have resolved.
///
/// WHAT: completes every retained argument through the session's shared call-argument service,
///       then infers the generic instance from the completed argument types and builds the value
///       through the constructor's own finisher.
/// WHY: the instance can be selected only after the argument leaves are concrete. Reusing the
///      immediate lane's inference and finisher keeps shape, defaults, routing and access unchanged.
pub(crate) fn complete_deferred_generic_nominal_call<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    deferred: DeferredGenericNominalCall,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredGenericNominalCall {
        nominal_path,
        diagnostic_name,
        mut seed,
        arguments,
        result_shape,
        call_span,
        constructor,
    } = deferred;

    // A slot's receiving type comes from this instance's own bindings, so a declared `?` reaches the
    // raw argument. Every instance edge settles and binds into the seed before any argument completes.
    bind_instance_edges(&mut seed, &nominal_path, result_shape, &mut *session)?;

    // Arguments complete before the session services are borrowed: both take the session mutably.
    let mut completed_arguments = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let argument = choose_slot_destination(argument, &constructor, &seed, &mut *session)?;
        let argument = choose_retained_argument_destination(argument, &mut *session)?;
        completed_arguments.push(session.complete_call_argument(argument)?);
    }

    let DeferredExpressionServices {
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = session.session_services();

    match constructor {
        GenericNominalConstructor::Struct {
            fields,
            struct_value_mode,
            enforce_const_record,
        } => complete_generic_struct_instance(
            GenericStructCompletion {
                nominal_path: &nominal_path,
                diagnostic_name,
                template_fields: &fields,
                struct_value_mode: &struct_value_mode,
                enforce_const_record,
                constructor_span: call_span,
                seed,
                arguments: completed_arguments,
            },
            context,
            type_interner,
            expression_builder,
            const_values,
            path_fork,
            string_table,
        ),
        GenericNominalConstructor::Choice {
            variant,
            payload_field_views,
            variant_index,
            variant_span,
            enforce_constant_context,
            route_future_input,
        } => {
            let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload else {
                return Err(CompilerError::compiler_error(
                    "a retained generic choice constructor's selected variant was not a payload variant",
                )
                .into());
            };
            if payload_field_views.len() != fields.len() {
                return Err(CompilerError::compiler_error(
                    "a retained generic choice constructor's payload views changed before completion",
                )
                .into());
            }
            if route_future_input == Some(true) && fields.is_empty() {
                return Err(CompilerError::compiler_error(
                    "a retained generic route constructor lost its future-input payload field",
                )
                .into());
            }
            let capture_field_count = fields.len() - usize::from(route_future_input == Some(true));
            complete_generic_choice_instance(
                GenericChoiceCompletion {
                    nominal_path: &nominal_path,
                    diagnostic_name,
                    template_fields: &fields[..capture_field_count],
                    payload_field_views: &payload_field_views[..capture_field_count],
                    variant_index,
                    variant_name: variant.name,
                    constructor_span: call_span,
                    variant_span,
                    enforce_constant_context,
                    route_future_input,
                    seed,
                    arguments: completed_arguments,
                },
                context,
                type_interner,
                expression_builder,
                const_values,
                path_fork,
                string_table,
            )
        }
    }
}

/// The declared type one argument slot of a retained constructor receives, before the instance's bindings apply.
///
/// WHAT: a struct's declared field type, or the declared payload field type of the selected choice variant.
///       A unit variant, or a slot past the declared fields, has no type.
/// WHY: the slots are the same declared views the immediate lane reads, so the receiving type is read by
///      index from them without building a second view list.
fn declared_slot_type(constructor: &GenericNominalConstructor, slot: usize) -> Option<TypeId> {
    match constructor {
        GenericNominalConstructor::Struct { fields, .. } => {
            fields.get(slot).map(|field| field.type_id)
        }
        GenericNominalConstructor::Choice {
            payload_field_views,
            ..
        } => payload_field_views.get(slot).map(|field| field.type_id),
    }
}

/// Settles and binds the instance's own binding edges into the seed's contextual bindings.
///
/// WHAT: the result shape is the declared self shape over the instance's bindings. A declared
///       substitution settles each edge's forward producers, materialises it as a consumer shape
///       does, and binds its parameter into the seed when no context binding names it. A concrete
///       result already fixes every parameter, so its instance arguments bind the same way.
/// WHY: the seed is the one binding map slots read receiving types from, so no second parameter map
///      exists. A context binding that conflicts with an edge stays, and argument inference reports
///      that conflict with its evidence.
fn bind_instance_edges<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    seed: &mut GenericNominalConstructorSeed,
    nominal_path: &PathId,
    result_shape: KnownShapeId,
    session: &mut S,
) -> Result<(), ExpressionParseError> {
    let shape = {
        let services = session.session_services();
        let state = services.context.numeric_state();
        state.shape(result_shape)
    };
    match shape {
        KnownShape::DeclaredSubstitution { bindings, .. } => {
            for index in bindings.indices() {
                let edge = {
                    let services = session.session_services();
                    let state = services.context.numeric_state();
                    state.declared_binding(index)
                };
                session.settle_forward_producers(edge.shape)?;
                let services = session.session_services();
                let mut state = services.context.numeric_state_mut();
                let type_id = materialise_known_shape(
                    &mut state,
                    services.type_interner.environment_mut_for_derived_types(),
                    edge.shape,
                )?;
                seed.bind_materialised_edge(edge.parameter, type_id);
            }
            Ok(())
        }
        KnownShape::Concrete { type_id, .. } => {
            bind_concrete_instance_arguments(seed, nominal_path, type_id, &mut *session)
        }
        KnownShape::Numeric(_)
        | KnownShape::Collection { .. }
        | KnownShape::Map { .. }
        | KnownShape::Optional { .. } => Err(CompilerError::compiler_error(
            "a retained constructor's result must be a declared substitution or concrete instance",
        )
        .into()),
    }
}

/// Binds a concrete instance's arguments into the seed's contextual bindings.
///
/// WHAT: each canonical parameter of the nominal takes the instance argument at its position. A
///       parameter the seed already names keeps its context type.
/// WHY: a seeded expected result or a ready argument can fix every parameter, so the envelope
///      collapses to the exact instance with no edges. Raw slots still read their receiving types
///      from the seed, so the instance's arguments must reach it here.
fn bind_concrete_instance_arguments<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    seed: &mut GenericNominalConstructorSeed,
    nominal_path: &PathId,
    instance_type_id: TypeId,
    session: &mut S,
) -> Result<(), ExpressionParseError> {
    let services = session.session_services();
    let environment = services.type_interner.environment();
    let Some(TypeDefinition::GenericInstance(instance)) = environment.get(instance_type_id) else {
        return Err(CompilerError::compiler_error(
            "a concrete retained constructor result must be a generic instance",
        )
        .into());
    };
    let Some(parameters) = environment.canonical_parameters_for_nominal(nominal_path) else {
        return Err(CompilerError::compiler_error(
            "a retained constructor's canonical parameters were missing",
        )
        .into());
    };
    if parameters.len() != instance.arguments.len() {
        return Err(CompilerError::compiler_error(
            "a concrete retained constructor's arguments did not match its parameters",
        )
        .into());
    }
    for (parameter, argument) in parameters.iter().zip(instance.arguments.iter()) {
        seed.bind_materialised_edge(parameter.id, *argument);
    }
    Ok(())
}

/// Gives a raw argument in a constructor slot the destination its declared type takes in this instance.
///
/// WHAT: a raw argument whose slot type, with the seed's bindings applied, has no unbound parameter
///       completes against that concrete receiving type. A declared `?` on the slot therefore reaches the
///       literal, which takes its one contextual wrap there.
/// WHY: a numeric origin keeps only its raw class, and the declared slot is the only record of the `?`.
///      Typed and forward arguments, and slots whose type still mentions an unbound parameter, pass through.
fn choose_slot_destination<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    argument: DeferredCallArgument,
    constructor: &GenericNominalConstructor,
    seed: &GenericNominalConstructorSeed,
    session: &mut S,
) -> Result<DeferredCallArgument, ExpressionParseError> {
    let (shape, mut expression) = match argument {
        DeferredCallArgument::Retained { shape, expression } => (shape, expression),
        other => return Ok(other),
    };

    let declared = shape
        .parameter_slot
        .and_then(|slot| declared_slot_type(constructor, slot.index()));
    if let Some(declared) = declared
        && matches!(expression.expected, ExpectedType::Infer)
    {
        let services = session.session_services();
        let environment = services.type_interner.environment_mut_for_derived_types();
        let receiving = environment.substitute_type_id(declared, seed.bindings().replacement_map());
        if !environment.contains_generic_parameter(receiving) {
            expression.expected = ExpectedType::Known(receiving);
        }
    }

    Ok(DeferredCallArgument::Retained { shape, expression })
}

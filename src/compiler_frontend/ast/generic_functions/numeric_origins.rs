//! Numeric-origin linking for retained generic signatures.
//!
//! WHAT: links one retained generic call's declared signature, or one retained generic nominal
//!       constructor's declared fields, to the numeric origins of its arguments. A free call also
//!       links its expected immediate result. Produces the symbolic result shapes and, for free
//!       calls, the error return, which is concrete only when the signature fixes it.
//! WHY: generic callables and generic nominal constructors share one signature-linking rule, so a
//!      declared parameter binds from the same argument evidence and projects through the same
//!      declared substitution whichever declaration owns it. A numeric literal passed to a generic
//!      parameter must keep its numeric class open until body completion resolves the whole
//!      expression. Binding the parameter from a default at call parse time would fix the class
//!      too early.

use crate::compiler_frontend::ast::expressions::call_argument::ParameterSlot;
use crate::compiler_frontend::ast::expressions::call_arguments::expected_type_for_resolved_parameter;
use crate::compiler_frontend::ast::expressions::constructor_views::ConstructorField;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::generic_functions::calls::GenericCallExpectedContext;
use crate::compiler_frontend::ast::generic_functions::diagnostics::cannot_infer_generic_function_arguments;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCallArgument, NumericConstructionState,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeBinding, KnownShapeBindingRange, KnownShapeId, declared_projection,
    register_shape_witness, union_shape_classes,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::GenericTypeBindings;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use rustc_hash::FxHashMap;

/// Borrowed inputs for linking one retained generic call's numeric signature.
///
/// WHAT: the template, the call's retained and ready arguments, the caller's immediate-result
///       evidence, and the interners needed to build symbolic shapes and diagnostics.
/// WHY: one input bundle keeps the link entry point from growing a long positional list.
pub(crate) struct GenericNumericSignatureInput<'a, 'b> {
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) arguments: &'a [DeferredCallArgument],
    pub(crate) expected_context: GenericCallExpectedContext<'a>,
    pub(crate) context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'b>,
    pub(crate) expression_builder: &'a AstExpressionBuilder,
    pub(crate) path_fork: &'a PathInternerFork,
    pub(crate) call_span: Option<SourceSpan>,
    /// Forward producers registered while this call's own argument text parsed. Only these pending
    /// producers take the immediate result's receiver, so an earlier binding's producers in the same
    /// numeric class never do.
    pub(crate) owned_producers: std::ops::Range<usize>,
    /// Known receiver shape of a method call. It binds the template's hidden receiver
    /// (`parameters[0]`) before authored arguments, which route with a slot offset of 1.
    /// `None` for free calls.
    pub(crate) known_receiver_shape: Option<KnownShapeId>,
}

/// The error return of one retained generic call.
///
/// WHAT: `Concrete` names an error type fixed by the signature alone. `Retained` names a shape
///       whose numeric leaves stay open until completion materialises the error type.
/// WHY: a numeric origin must never choose the error role at parse time, or freeze a class shared
///      with a success result or named origin. Only completion may materialise a retained error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenericErrorReturn {
    Concrete(TypeId),
    Retained(KnownShapeId),
}

/// The symbolic signature of one retained generic call.
///
/// WHAT: one result shape per success return, plus the error return when the signature declares one.
/// WHY: result shapes keep numeric origins open until body completion resolves them.
pub(crate) struct GenericNumericSignature {
    pub(crate) result_shapes: Vec<KnownShapeId>,
    pub(crate) error_return: Option<GenericErrorReturn>,
}

/// Link a retained generic call's numeric origins to its declared signature.
///
/// WHAT: binds the template's generic parameters from argument evidence (type evidence from ready
///       arguments, shape evidence from retained origins), reconciles shapes with concrete
///       receivers, applies immediate-result evidence (a shape-bound parameter takes the expected
///       type as a receiving witness, a free parameter takes it as its type), then builds the
///       symbolic result shapes and the error return.
///       A known method receiver binds the hidden `parameters[0]` before authored arguments.
/// WHY: numeric completion needs each result's shape before body completion, but must never
///      default a numeric class at parse time.
/// MUST NOT: be called while another `numeric_state` borrow is held; it takes `numeric_state_mut`.
pub(crate) fn link_generic_numeric_signature(
    input: GenericNumericSignatureInput<'_, '_>,
) -> Result<GenericNumericSignature, ExpressionParseError> {
    let GenericNumericSignatureInput {
        template,
        arguments,
        expected_context,
        context,
        type_interner,
        expression_builder,
        path_fork,
        call_span,
        owned_producers,
        known_receiver_shape,
    } = input;

    let environment = type_interner.environment_mut_for_derived_types();
    let mut state = context.numeric_state_mut();
    let parameters = environment
        .generic_parameters(template.generic_parameter_list_id)
        .map(|list| {
            list.parameters
                .iter()
                .map(|parameter| (parameter.id, parameter.name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let receiver_slot_offset = usize::from(known_receiver_shape.is_some());

    let mut linker = NumericSignatureLinker {
        environment,
        state: &mut state,
        parameters,
        type_bindings: GenericTypeBindings::new(),
        shape_bindings: FxHashMap::default(),
        call_span,
    };

    if let Some(receiver_shape) = known_receiver_shape {
        // The receiver is the template's first declared parameter, so it takes no offset.
        let declared = declared_parameter_type(template, ParameterSlot::new(0), 0)?;
        linker.bind_shape(declared, receiver_shape)?;
    }

    for argument in arguments {
        match argument {
            DeferredCallArgument::Ready(call) => {
                // An argument with no declared slot carries no generic evidence; ordinary
                // validation reports any routing problem.
                let Some(slot) = call.parameter_slot else {
                    continue;
                };
                let declared = declared_parameter_type(template, slot, receiver_slot_offset)?;
                let type_id = expression_builder
                    .view(call.value)?
                    .require_single(ValueReceiverContext::FunctionArgument)?;
                linker.bind_type(declared, type_id);
            }
            DeferredCallArgument::Retained { shape, .. }
            | DeferredCallArgument::Forward { shape, .. } => {
                // A retained argument without a generic-linked origin carries no shape evidence.
                let (Some(slot), Some(origin)) = (shape.parameter_slot, shape.origin_shape) else {
                    continue;
                };
                let declared = declared_parameter_type(template, slot, receiver_slot_offset)?;
                linker.bind_shape(declared, origin)?;
            }
        }
    }
    linker.reconcile_witnesses();

    let returns = template.signature.success_return_type_ids();
    if let GenericCallExpectedContext::ImmediateResult(expected) = expected_context {
        linker.apply_expected_results(&returns, expected, &owned_producers)?;
    }

    let missing = linker.unbound_parameter_names();
    if !missing.is_empty() {
        return Err(cannot_infer_generic_function_arguments(
            path_fork.component(template.function_path),
            missing,
            call_span,
        )
        .into());
    }

    linker.complete_type_parameter_shapes()?;

    let result_shapes = returns
        .iter()
        .map(|declared| linker.envelope(*declared))
        .collect::<Result<Vec<_>, _>>()?;
    let error_return = linker.error_return(template.signature.error_return_type_id())?;

    Ok(GenericNumericSignature {
        result_shapes,
        error_return,
    })
}

/// Borrowed inputs for linking one retained generic nominal constructor's result shape.
///
/// WHAT: the nominal identity, its declared field views, the ready and retained arguments, the
///       seeded contextual bindings, and the interners needed for the declared self shape and the
///       missing-parameter diagnostic.
/// WHY: a retained constructor needs its symbolic result shape before completion, and it must reuse
///      the nominal's own parameters and field views rather than building a second template.
pub(crate) struct GenericNominalShapeInput<'a, 'b> {
    pub(crate) nominal_path: &'a PathId,
    pub(crate) display_name: &'a str,
    pub(crate) declared_fields: &'a [ConstructorField],
    pub(crate) arguments: &'a [DeferredCallArgument],
    pub(crate) seeded_bindings: &'a GenericTypeBindings,
    pub(crate) context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'b>,
    pub(crate) expression_builder: &'a AstExpressionBuilder,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) call_span: Option<SourceSpan>,
}

/// Link a retained generic nominal constructor's numeric origins to its declared self shape.
///
/// WHAT: binds the nominal's parameters from the seeded contextual bindings, ready argument types
///       and retained origin shapes, then envelopes the declared self instance over its parameters.
/// WHY: a retained constructor's consumer needs one symbolic result shape before completion. The
///      declared self instance is the existing `KnownShape::DeclaredSubstitution` identity, so no
///      second nominal shape system or fake function template is introduced.
/// MUST NOT: be called while another `numeric_state` borrow is held; it takes `numeric_state_mut`.
pub(crate) fn link_generic_nominal_result_shape(
    input: GenericNominalShapeInput<'_, '_>,
) -> Result<KnownShapeId, ExpressionParseError> {
    let GenericNominalShapeInput {
        nominal_path,
        display_name,
        declared_fields,
        arguments,
        seeded_bindings,
        context,
        type_interner,
        expression_builder,
        string_table,
        call_span,
    } = input;

    let Some(canonical_parameters) = type_interner
        .environment()
        .canonical_parameters_for_nominal(nominal_path)
    else {
        return Err(CompilerError::compiler_error(format!(
            "Generic nominal '{display_name}' has no canonical parameter list after registration"
        ))
        .into());
    };
    let parameters: Vec<(GenericParameterId, StringId)> = canonical_parameters
        .iter()
        .map(|parameter| (parameter.id, parameter.name))
        .collect();

    let Some(nominal_id) = type_interner
        .environment()
        .nominal_id_for_path(nominal_path)
    else {
        return Err(CompilerError::compiler_error(
            "a retained generic nominal constructor's declaration was missing from the type environment",
        )
        .into());
    };
    let mut parameter_type_ids = Vec::with_capacity(parameters.len());
    for (parameter_id, _) in &parameters {
        let Some(type_id) = type_interner
            .environment()
            .type_id_for_generic_parameter(*parameter_id)
        else {
            return Err(CompilerError::compiler_error(
                "a retained generic nominal constructor's parameter had no canonical TypeId",
            )
            .into());
        };
        parameter_type_ids.push(type_id);
    }
    let declared_self =
        type_interner.intern_generic_instance(nominal_id, parameter_type_ids.into_boxed_slice());

    let mut state = context.numeric_state_mut();
    let mut linker = NumericSignatureLinker {
        environment: type_interner.environment_mut_for_derived_types(),
        state: &mut state,
        parameters,
        type_bindings: seeded_bindings.clone(),
        shape_bindings: FxHashMap::default(),
        call_span,
    };

    for argument in arguments {
        match argument {
            DeferredCallArgument::Ready(call) => {
                // An argument with no declared field carries no generic evidence; ordinary
                // validation at completion reports any routing problem.
                let Some(slot) = call.parameter_slot else {
                    continue;
                };
                let Some(field) = declared_fields.get(slot.index()) else {
                    continue;
                };
                let type_id = expression_builder
                    .view(call.value)?
                    .require_single(ValueReceiverContext::ConstructorArgument)?;
                linker.bind_type(field.type_id, type_id);
            }
            DeferredCallArgument::Retained { shape, .. }
            | DeferredCallArgument::Forward { shape, .. } => {
                let (Some(slot), Some(origin)) = (shape.parameter_slot, shape.origin_shape) else {
                    continue;
                };
                let Some(field) = declared_fields.get(slot.index()) else {
                    continue;
                };
                linker.bind_shape(field.type_id, origin)?;
            }
        }
    }
    linker.reconcile_witnesses();

    // Missing parameters are the ordinary inference error, matching the immediate constructor lane.
    let missing = linker.unbound_parameter_names();
    if !missing.is_empty() {
        return Err(CompilerDiagnostic::invalid_generic_instantiation(
            Some(string_table.intern(display_name)),
            InvalidGenericInstantiationReason::CannotInferArguments {
                missing_parameters: missing,
            },
            call_span,
        )
        .into());
    }

    linker.complete_type_parameter_shapes()?;
    let shape = linker.envelope(declared_self)?;
    Ok(shape)
}

/// Mutable linking state for one call.
///
/// WHAT: the borrowed type environment and numeric state, the parameter list in declaration
///       order, the type and shape evidence gathered so far, and the call span for witnesses.
/// WHY: declaration-order parameters keep witness registration and diagnostics deterministic.
struct NumericSignatureLinker<'a> {
    environment: &'a mut TypeEnvironment,
    state: &'a mut NumericConstructionState,
    parameters: Vec<(GenericParameterId, StringId)>,
    type_bindings: GenericTypeBindings,
    shape_bindings: FxHashMap<GenericParameterId, KnownShapeId>,
    call_span: Option<SourceSpan>,
}

impl NumericSignatureLinker<'_> {
    fn is_bound(&self, parameter: GenericParameterId) -> bool {
        self.shape_bindings.contains_key(&parameter) || self.type_bindings.get(parameter).is_some()
    }

    fn unbound_parameter_names(&self) -> Vec<StringId> {
        self.parameters
            .iter()
            .filter(|(parameter, _)| !self.is_bound(*parameter))
            .map(|(_, name)| *name)
            .collect()
    }

    /// Bind type evidence from a completed argument.
    ///
    /// A structural mismatch or binding conflict is not reported here; completion's ordinary
    /// argument validation reports it against the bound signature.
    fn bind_type(&mut self, declared: TypeId, actual: TypeId) {
        if !self.environment.contains_generic_parameter(declared) {
            return;
        }
        let _ = self.environment.try_collect_type_parameter_bindings_typeid(
            declared,
            actual,
            &mut self.type_bindings,
        );
    }

    /// Whether this origin's value fills one declared optional layer inside `bind_shape`.
    ///
    /// WHAT: numeric, collection, map and declared-substitution origins are present shape binders
    ///       whose one declared optional layer is peeled here. An `Optional` origin consumes its
    ///       own layer through recursion. A concrete origin wraps through the shared type collector.
    /// WHY: each present value fills exactly one declared optional layer per structural position.
    ///      Peeling a concrete origin here would wrap the same value a second time in the collector.
    fn peels_declared_option(&self, shape: KnownShapeId) -> bool {
        match self.state.shape(shape) {
            KnownShape::Concrete { .. } | KnownShape::Optional { .. } => false,
            KnownShape::Numeric(_)
            | KnownShape::Collection { .. }
            | KnownShape::Map { .. }
            | KnownShape::DeclaredSubstitution { .. } => true,
        }
    }

    /// Bind shape evidence from a retained or forwarded argument's numeric origin.
    ///
    /// WHAT: a numeric, collection, map or declared-substitution origin fills one declared
    ///       optional layer, so that layer is peeled once before matching. A bare parameter
    ///       receives the shape itself, so its numeric class stays open. A concrete shape binds
    ///       through the shared type walk, which wraps it once. An optional origin matches its own
    ///       layer through recursion.
    /// WHY: `T?` accepts a present `T` by contextual wrapping, so the parameter must read the
    ///      origin's own class rather than an optional layer the origin does not carry.
    fn bind_shape(
        &mut self,
        declared: TypeId,
        shape: KnownShapeId,
    ) -> Result<(), ExpressionParseError> {
        if !self.environment.contains_generic_parameter(declared) {
            return Ok(());
        }
        let declared = self.peeled_declared(declared, shape);
        self.bind_shape_layer(declared, shape)
    }

    /// Peel the declared optional layer that a present origin shape fills.
    ///
    /// WHY: each present origin value fills one declared optional layer, so it is peeled once here
    ///      and shared by shape binding.
    fn peeled_declared(&self, declared: TypeId, shape: KnownShapeId) -> TypeId {
        match self.environment.option_inner_type(declared) {
            Some(inner) if self.peels_declared_option(shape) => inner,
            _ => declared,
        }
    }

    /// Bind shape evidence against one declared layer that has already been peeled.
    ///
    /// WHY: a peeled layer is never peeled again for the same origin layer. An optional origin's
    ///      inner shape binds through its own recursion, so each declared optional layer is filled
    ///      by exactly one origin layer.
    fn bind_shape_layer(
        &mut self,
        declared: TypeId,
        shape: KnownShapeId,
    ) -> Result<(), ExpressionParseError> {
        if let Some(parameter) = self.environment.bare_generic_parameter(declared) {
            self.bind_shape_parameter(parameter, shape);
            return Ok(());
        }
        match self.state.shape(shape) {
            KnownShape::Concrete { type_id, .. } => self.bind_type(declared, type_id),
            // A numeric leaf is not a bindable concrete type; ordinary validation reports it.
            KnownShape::Numeric(_) => {}
            KnownShape::Optional { inner } => {
                if let Some(inner_declared) = self.environment.option_inner_type(declared) {
                    self.bind_shape(inner_declared, inner)?;
                }
            }
            KnownShape::Collection {
                element,
                fixed_capacity,
            } => {
                if let Some(collection) = self
                    .environment
                    .collection_shape(declared)
                    .filter(|collection| collection.fixed_capacity == fixed_capacity)
                {
                    self.bind_shape(collection.element_type, element)?;
                }
            }
            KnownShape::Map { key, value } => {
                if let Some(map) = self.environment.map_shape(declared) {
                    self.bind_shape(map.key_type, key)?;
                    self.bind_shape(map.value_type, value)?;
                }
            }
            KnownShape::DeclaredSubstitution {
                declared_type_id,
                bindings,
            } => {
                self.bind_declared_substitution(declared, declared_type_id, bindings)?;
            }
        }
        Ok(())
    }

    /// Bind the parameters a declared envelope mentions from a declared-substitution origin.
    ///
    /// WHAT: an identical declaration pairs each mentioned parameter with its stored edge. A
    ///       different declaration projects its parameters onto the origin's declared type, then
    ///       builds each projected binding's shape from the origin's edges.
    /// WHY: the origin keeps its numeric classes open through its declaration, so binding must
    ///      follow that declaration and never read the origin's parameter identities as concrete.
    fn bind_declared_substitution(
        &mut self,
        declared: TypeId,
        origin_declared: TypeId,
        origin: KnownShapeBindingRange,
    ) -> Result<(), ExpressionParseError> {
        let mut mentioned = Vec::new();
        self.environment
            .collect_generic_parameters(declared, &mut mentioned);
        if declared == origin_declared {
            for parameter in mentioned {
                if let Some(shape) = self.state.declared_binding_shape(origin, parameter) {
                    self.bind_shape_parameter(parameter, shape);
                }
            }
            return Ok(());
        }
        let Some(projected) = declared_projection(&*self.environment, declared, origin_declared)
        else {
            return Ok(());
        };
        for parameter in mentioned {
            let Some(bound) = projected.get(parameter) else {
                continue;
            };
            let shape = envelope_shape(
                &mut *self.state,
                &mut *self.environment,
                bound,
                ParameterShapes::Range(origin),
                self.call_span,
            )?;
            self.bind_shape_parameter(parameter, shape);
        }
        Ok(())
    }

    /// Record one shape per parameter, joining later shapes into the first one's class.
    ///
    /// A refused join leaves the pair separate; completion coercion reports it against each side.
    fn bind_shape_parameter(&mut self, parameter: GenericParameterId, shape: KnownShapeId) {
        match self.shape_bindings.get(&parameter).copied() {
            None => {
                self.shape_bindings.insert(parameter, shape);
            }
            Some(existing) => {
                let _joined =
                    union_shape_classes(&mut *self.state, &*self.environment, existing, shape);
            }
        }
    }

    /// Register each shape-bound parameter's concrete receiver witness, in declaration order.
    fn reconcile_witnesses(&mut self) {
        for &(parameter, _) in &self.parameters {
            let (Some(shape), Some(type_id)) = (
                self.shape_bindings.get(&parameter).copied(),
                self.type_bindings.get(parameter),
            ) else {
                continue;
            };
            register_shape_witness(
                &mut *self.state,
                &*self.environment,
                shape,
                type_id,
                self.call_span,
            );
        }
    }

    /// Apply the caller's immediate-result evidence to the call's parameters.
    ///
    /// WHAT: stages each expected result against its success return type. A shape-bound parameter
    ///       takes the expected type as a receiving witness on its numeric class. A free parameter
    ///       takes it as its type. A type-bound parameter keeps its argument evidence, and ordinary
    ///       validation checks the expected result against it.
    /// WHY: a numeric origin must resolve to the caller's expected type rather than to a family
    ///      default, while an argument's own type evidence stays authoritative.
    fn apply_expected_results(
        &mut self,
        returns: &[TypeId],
        expected: &[TypeId],
        owned_producers: &std::ops::Range<usize>,
    ) -> Result<(), ExpressionParseError> {
        if returns.len() != expected.len() {
            return Ok(());
        }
        let mut expected_bindings = GenericTypeBindings::new();
        for (declared, actual) in returns.iter().zip(expected) {
            if !self.environment.contains_generic_parameter(*declared) {
                continue;
            }
            // A conflicting expected result stays unbound here; ordinary validation reports it.
            let _ = self.environment.try_collect_type_parameter_bindings_typeid(
                *declared,
                *actual,
                &mut expected_bindings,
            );
        }
        for &(parameter, _) in &self.parameters {
            let Some(expected_type) = expected_bindings.get(parameter) else {
                continue;
            };
            match (
                self.shape_bindings.get(&parameter).copied(),
                self.type_bindings.get(parameter),
            ) {
                // Argument type evidence is complete; ordinary validation checks the expectation.
                (_, Some(_)) => {}
                (Some(shape), None) => {
                    register_shape_witness(
                        &mut *self.state,
                        &*self.environment,
                        shape,
                        expected_type,
                        self.call_span,
                    );
                    // The immediate result solves this parameter, so its pending forward leaves take
                    // that solution as their literal receiver; later class witnesses never do.
                    set_forward_destinations(
                        &mut *self.state,
                        &*self.environment,
                        shape,
                        expected_type,
                        owned_producers,
                    );
                }
                (None, None) => {
                    self.type_bindings
                        .insert_consistent(parameter, expected_type)
                        .map_err(|_| {
                            infrastructure(
                                "expected generic result conflicted with an unbound parameter",
                            )
                        })?;
                }
            }
        }
        Ok(())
    }

    /// Give each type-bound parameter a concrete shape, so envelopes read every parameter alike.
    fn complete_type_parameter_shapes(&mut self) -> Result<(), ExpressionParseError> {
        for &(parameter, _) in &self.parameters {
            if self.shape_bindings.contains_key(&parameter) {
                continue;
            }
            let Some(type_id) = self.type_bindings.get(parameter) else {
                continue;
            };
            let shape = self.state.intern_shape(KnownShape::Concrete {
                type_id,
                span: self.call_span,
            })?;
            self.shape_bindings.insert(parameter, shape);
        }
        Ok(())
    }

    /// Build the symbolic shape of one declared success return from the call's parameter shapes.
    fn envelope(&mut self, declared: TypeId) -> Result<KnownShapeId, ExpressionParseError> {
        envelope_shape(
            &mut *self.state,
            &mut *self.environment,
            declared,
            ParameterShapes::Map(&self.shape_bindings),
            self.call_span,
        )
    }

    /// Build the error return from the declared error type.
    ///
    /// WHAT: a concrete envelope is the error type itself. Any envelope with a numeric or forward
    ///       leaf stays a retained shape, to be verified against the completed call.
    /// WHY: a numeric origin must never choose the error role at parse time.
    fn error_return(
        &mut self,
        declared: Option<TypeId>,
    ) -> Result<Option<GenericErrorReturn>, ExpressionParseError> {
        let Some(declared) = declared else {
            return Ok(None);
        };
        let shape = self.envelope(declared)?;
        Ok(Some(match self.state.shape(shape) {
            KnownShape::Concrete { type_id, .. } => GenericErrorReturn::Concrete(type_id),
            _ => GenericErrorReturn::Retained(shape),
        }))
    }
}

/// Give pending forward leaves of a shape-bound argument the immediate result's receiver.
///
/// WHAT: walks the argument's known shape beside the solved expected type. Each numeric leaf gives
///       its pending forward producers registered in `owned_producers` the literal destination;
///       collections, maps and optionals descend to their elements, values and inner shapes. A
///       declared substitution descends through the edges its declared type binds from the expected
///       type. Concrete shapes stop.
/// WHY: a forward producer nested in a collection, map or optional is reached only through its
///      argument's shape, so one shape walk replaces per-parameter producer lists. Restricting it to
///      the producers its own argument text registered keeps an earlier binding's producer on its
///      natural result, even when it shares the numeric class.
pub(crate) fn set_forward_destinations(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    shape: KnownShapeId,
    expected: TypeId,
    owned_producers: &std::ops::Range<usize>,
) {
    // A present structured value fills one declared optional layer, exactly as witness
    // registration peels it, so structured arms read the present receiver.
    let present = environment.option_inner_type(expected).unwrap_or(expected);
    match state.shape(shape) {
        KnownShape::Numeric(class) => {
            let destination = expected_type_for_resolved_parameter(expected, environment);
            if matches!(destination, ExpectedType::NumericLiteral(_)) {
                state.set_pending_forward_destinations(class, destination, owned_producers.clone());
            }
        }
        KnownShape::Collection { element, .. } => {
            if let Some(collection) = environment.collection_shape(present) {
                set_forward_destinations(
                    state,
                    environment,
                    element,
                    collection.element_type,
                    owned_producers,
                );
            }
        }
        KnownShape::Map { key, value } => {
            if let Some(map) = environment.map_shape(present) {
                set_forward_destinations(state, environment, key, map.key_type, owned_producers);
                set_forward_destinations(
                    state,
                    environment,
                    value,
                    map.value_type,
                    owned_producers,
                );
            }
        }
        KnownShape::Optional { inner } => {
            set_forward_destinations(state, environment, inner, present, owned_producers);
        }
        KnownShape::DeclaredSubstitution {
            declared_type_id,
            bindings,
        } => {
            // WHY: a declared substitution keeps its parameters open. The expected nominal fixes
            //      each parameter's type, and each edge's shape receives that type.
            let mut expected_bindings = GenericTypeBindings::new();
            if let Ok(true) = environment.try_collect_type_parameter_bindings_typeid(
                declared_type_id,
                present,
                &mut expected_bindings,
            ) {
                for index in bindings.indices() {
                    let edge = state.declared_binding(index);
                    if let Some(bound) = expected_bindings.get(edge.parameter) {
                        set_forward_destinations(
                            state,
                            environment,
                            edge.shape,
                            bound,
                            owned_producers,
                        );
                    }
                }
            }
        }
        KnownShape::Concrete { .. } => {}
    }
}

/// The parameter shapes an envelope reads from.
///
/// WHAT: either the call's own parameter map, or the stored edges of a declared-substitution origin.
/// WHY: one envelope builder serves both the call's signature and origin projection, so the two
///      cannot disagree about which structural positions stay open.
#[derive(Clone, Copy)]
enum ParameterShapes<'a> {
    Map(&'a FxHashMap<GenericParameterId, KnownShapeId>),
    Range(KnownShapeBindingRange),
}

impl ParameterShapes<'_> {
    fn lookup(
        self,
        state: &NumericConstructionState,
        parameter: GenericParameterId,
    ) -> Option<KnownShapeId> {
        match self {
            ParameterShapes::Map(shapes) => shapes.get(&parameter).copied(),
            ParameterShapes::Range(range) => state.declared_binding_shape(range, parameter),
        }
    }

    fn missing_binding(self) -> ExpressionParseError {
        match self {
            ParameterShapes::Map(_) => {
                infrastructure("generic result parameter has no binding after the unbound check")
            }
            ParameterShapes::Range(_) => infrastructure(
                "declared substitution is missing a binding for a parameter its type mentions",
            ),
        }
    }
}

/// Build the envelope shape of one declared type from the shapes its parameters resolve to.
///
/// WHAT: a type whose parameters all resolve concretely collapses to its concrete substitution.
///       An option, collection, or map with a numeric-dependent parameter keeps its structure.
///       Every other numeric-dependent declared type becomes one declared-substitution node over
///       its parameter edges.
/// WHY: a numeric class stays open in every position that reaches it, and no structural form is
///      chosen by a default. Fully concrete envelopes keep their exact canonical type.
fn envelope_shape(
    state: &mut NumericConstructionState,
    environment: &mut TypeEnvironment,
    declared: TypeId,
    parameters: ParameterShapes<'_>,
    span: Option<SourceSpan>,
) -> Result<KnownShapeId, ExpressionParseError> {
    if let Some(parameter) = environment.bare_generic_parameter(declared) {
        return parameters
            .lookup(&*state, parameter)
            .ok_or_else(|| parameters.missing_binding());
    }

    // Structural forms decide concreteness from their already processed children, so a nested
    // envelope is walked once rather than rescanned for generic parameters at every level.
    // Concrete children already supply the canonical constructor arguments, so neither branch
    // repeats the substitution walk.
    if let Some(inner) = environment.option_inner_type(declared) {
        let inner = envelope_shape(state, environment, inner, parameters, span)?;
        return match state.shape(inner) {
            KnownShape::Concrete { type_id, .. } => {
                let type_id = environment.intern_option(type_id);
                state.intern_shape(KnownShape::Concrete { type_id, span })
            }
            _ => state.intern_shape(KnownShape::Optional { inner }),
        };
    }
    if let Some(collection) = environment.collection_shape(declared) {
        let element = envelope_shape(
            state,
            environment,
            collection.element_type,
            parameters,
            span,
        )?;
        return match state.shape(element) {
            KnownShape::Concrete { type_id, .. } => {
                let type_id = environment.intern_collection(type_id, collection.fixed_capacity);
                state.intern_shape(KnownShape::Concrete { type_id, span })
            }
            _ => state.intern_shape(KnownShape::Collection {
                element,
                fixed_capacity: collection.fixed_capacity,
            }),
        };
    }
    if let Some(map) = environment.map_shape(declared) {
        let key = envelope_shape(state, environment, map.key_type, parameters, span)?;
        let value = envelope_shape(state, environment, map.value_type, parameters, span)?;
        return match (state.shape(key), state.shape(value)) {
            (
                KnownShape::Concrete {
                    type_id: key_type, ..
                },
                KnownShape::Concrete {
                    type_id: value_type,
                    ..
                },
            ) => {
                let type_id = environment.intern_map(key_type, value_type);
                state.intern_shape(KnownShape::Concrete { type_id, span })
            }
            _ => state.intern_shape(KnownShape::Map { key, value }),
        };
    }

    if !environment.contains_generic_parameter(declared) {
        return state.intern_shape(KnownShape::Concrete {
            type_id: declared,
            span,
        });
    }

    let mut mentioned = Vec::new();
    environment.collect_generic_parameters(declared, &mut mentioned);
    let mut edges = Vec::with_capacity(mentioned.len());
    let mut mapping = FxHashMap::default();
    let mut all_concrete = true;
    for parameter in mentioned {
        let shape = parameters
            .lookup(&*state, parameter)
            .ok_or_else(|| parameters.missing_binding())?;
        match state.shape(shape) {
            KnownShape::Concrete { type_id, .. } => {
                mapping.insert(parameter, type_id);
            }
            _ => all_concrete = false,
        }
        edges.push(KnownShapeBinding { parameter, shape });
    }
    if all_concrete {
        let concrete = environment.substitute_type_id(declared, &mapping);
        return state.intern_shape(KnownShape::Concrete {
            type_id: concrete,
            span,
        });
    }

    state.intern_declared_substitution(declared, &edges)
}

/// Project one declared struct field through a declared-substitution origin's bindings.
///
/// WHAT: finds the named field on the declared struct, or on the base struct of a generic
///       instance. A generic instance first substitutes its arguments for the base parameters.
///       The field's type then envelopes through `range`, keeping its generic, non-numeric, and
///       capacity facts and its authored span.
/// WHY: a retained `origin.field` must not collapse to an ordinary field read before its numeric
///      origin settles.
/// MUST NOT: report visibility or member errors. A missing field, or an owner that is not a
///      struct, returns `None` so the ordinary field finisher still owns those diagnostics.
/// MUST NOT: accept choice payload fields. Choice payloads project through `declared_choice_payload_shape`.
pub(crate) fn declared_field_shape(
    state: &mut NumericConstructionState,
    environment: &mut TypeEnvironment,
    path_fork: &PathInternerFork,
    declared_owner: TypeId,
    field_name: StringId,
    range: KnownShapeBindingRange,
    span: Option<SourceSpan>,
) -> Result<Option<KnownShapeId>, ExpressionParseError> {
    // Every environment read ends before substitution and envelope interning take `&mut` access.
    let (declared_field_type, field_span, mapping) = {
        let (definition_id, instance) = match environment.get(declared_owner) {
            Some(TypeDefinition::Struct(definition)) => (definition.id, None),
            Some(TypeDefinition::GenericInstance(instance)) => (instance.base, Some(instance)),
            _ => return Ok(None),
        };
        let Some(definition) = environment.struct_definition(definition_id) else {
            return Ok(None);
        };
        let Some(field) = definition
            .fields
            .iter()
            .find(|field| path_fork.component(field.name) == Some(field_name))
        else {
            return Ok(None);
        };
        let mapping = match instance {
            None => None,
            Some(instance) => {
                let parameters = definition
                    .generic_parameters
                    .and_then(|list| environment.generic_parameters(list));
                let parameter_count = parameters.map_or(0, |list| list.parameters.len());
                if parameter_count != instance.arguments.len() {
                    return Err(infrastructure(
                        "generic instance arguments do not match their base struct's parameters",
                    ));
                }
                let mut mapping: FxHashMap<GenericParameterId, TypeId> = FxHashMap::default();
                if let Some(list) = parameters {
                    for (parameter, argument) in
                        list.parameters.iter().zip(instance.arguments.iter())
                    {
                        mapping.insert(parameter.id, *argument);
                    }
                }
                Some(mapping)
            }
        };
        (field.type_id, field.span.or(span), mapping)
    };

    let field_type = match &mapping {
        Some(mapping) => environment.substitute_type_id(declared_field_type, mapping),
        None => declared_field_type,
    };
    envelope_shape(
        state,
        environment,
        field_type,
        ParameterShapes::Range(range),
        field_span,
    )
    .map(Some)
}

/// A declared generic choice behind one retained subject shape, with the bindings that map its
/// parameters.
///
/// WHY: a pending `match` must learn variant identity at parse time, before the subject's concrete
///      instance is selected, so the declared owner and its binding range travel together.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DeclaredChoiceSubject {
    pub(crate) declared_type_id: TypeId,
    pub(crate) bindings: KnownShapeBindingRange,
}

/// Select the declared generic choice a retained subject shape names, when it names one.
///
/// WHAT: a declared-substitution shape whose owner has choice variants answers its owner and
///       binding range. Every other shape, including a struct owner, answers `None`.
/// WHY: only a choice owner changes the pattern path; structs keep the ordinary non-choice path.
pub(crate) fn declared_choice_subject(
    state: &NumericConstructionState,
    environment: &TypeEnvironment,
    shape: KnownShapeId,
) -> Option<DeclaredChoiceSubject> {
    let KnownShape::DeclaredSubstitution {
        declared_type_id,
        bindings,
    } = state.shape(shape)
    else {
        return None;
    };
    environment
        .variants_for(declared_type_id)
        .map(|_| DeclaredChoiceSubject {
            declared_type_id,
            bindings,
        })
}

/// Select the inner shape of a retained optional subject, when the subject is optional.
///
/// WHAT: an optional shape answers its inner shape. Every other shape answers `None`.
/// WHY: a pending `match` on an optional subject reads the payload shape at parse time, so the
///      arm can capture it before the subject's concrete optional type is selected.
pub(crate) fn optional_subject_inner(
    state: &NumericConstructionState,
    shape: KnownShapeId,
) -> Option<KnownShapeId> {
    match state.shape(shape) {
        KnownShape::Optional { inner } => Some(inner),
        KnownShape::Concrete { .. }
        | KnownShape::Numeric(_)
        | KnownShape::Collection { .. }
        | KnownShape::Map { .. }
        | KnownShape::DeclaredSubstitution { .. } => None,
    }
}

/// Project one declared choice payload field through a declared-substitution subject's bindings.
///
/// WHAT: finds the variant with `variant_tag` on the declared choice, reads record field
///       `field_index`, and envelopes its declared type through the subject's bindings.
/// WHY: a `Some(value as payload)` capture of a retained generic subject reads the same parameter
///      class as the origin's constructor argument, so a later read can witness that class.
/// MUST NOT: accept a unit variant or a missing field; the pattern parser has already checked both.
pub(crate) fn declared_choice_payload_shape(
    state: &mut NumericConstructionState,
    environment: &mut TypeEnvironment,
    subject: DeclaredChoiceSubject,
    variant_tag: usize,
    field_index: usize,
    span: Option<SourceSpan>,
) -> Result<KnownShapeId, ExpressionParseError> {
    // The variant read ends before envelope interning takes `&mut` access to the environment.
    let (field_type, field_span) = {
        let variants = environment
            .variants_for(subject.declared_type_id)
            .ok_or_else(|| infrastructure("retained choice subject has no declared variants"))?;
        let variant = variants
            .iter()
            .find(|variant| variant.tag == variant_tag)
            .ok_or_else(|| {
                infrastructure(
                    "retained choice payload names a variant tag the declared choice lacks",
                )
            })?;
        let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload else {
            return Err(infrastructure(
                "retained choice payload capture targets a unit variant",
            ));
        };
        let field = fields.get(field_index).ok_or_else(|| {
            infrastructure("retained choice payload capture names a missing field")
        })?;
        (field.type_id, field.span.or(span))
    };
    envelope_shape(
        state,
        environment,
        field_type,
        ParameterShapes::Range(subject.bindings),
        field_span,
    )
}

/// Resolve a retained or ready argument's declared parameter type from the template signature.
///
/// WHAT: the parser-selected slot is authored-relative. `receiver_slot_offset` maps it past a
///       hidden leading receiver to the template's declaration-order parameter.
/// WHY: slots are the single routing truth; linking must not rematch argument names.
fn declared_parameter_type(
    template: &GenericFunctionTemplate,
    slot: ParameterSlot,
    receiver_slot_offset: usize,
) -> Result<TypeId, ExpressionParseError> {
    slot.index()
        .checked_add(receiver_slot_offset)
        .and_then(|index| template.signature.parameters.get(index))
        .map(|parameter| parameter.type_id)
        .ok_or_else(|| {
            infrastructure("generic call argument routed outside the template signature")
        })
}

fn infrastructure(message: &str) -> ExpressionParseError {
    ExpressionParseError::Infrastructure(Box::new(CompilerError::compiler_error(message)))
}

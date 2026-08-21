//! Shared call-argument normalization and validation.
//!
//! WHAT: consumes parser-retained argument slots, fills defaults and enforces the shared rules for
//! type compatibility, reactive-source requirements and explicit access mode.
//! WHY: function calls, struct constructors, receiver methods and builtin members all need the
//! same final argument policy even though they build different AST nodes afterward. Named and
//! positional syntax is owned by `call_arguments` and is not reconstructed here.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode, ParameterSlot,
    order_call_arguments_by_retained_slot,
};
use crate::compiler_frontend::ast::expressions::constructor_views::{
    ConstructorField, ConstructorFieldAccessMode,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCallShapeReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;

use crate::compiler_frontend::external_packages::{
    ExternalAccessKind, ExternalFunctionDef, ExternalParameter,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::type_coercion::compatibility::{
    TypeCompatibilityCache, TypeCompatibilityMode,
};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_declared_type;

pub(crate) enum CallValidationError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for CallValidationError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        CallValidationError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<CompilerError> for CallValidationError {
    fn from(error: CompilerError) -> Self {
        CallValidationError::Infrastructure(Box::new(error))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CallSurfaceKind {
    Function,
    StructConstructor,
    ChoiceConstructor,
    ReceiverMethod,
    BuiltinMember,
    HostFunction,
    Assertion,
}

#[derive(Clone, Copy)]
pub(crate) struct CallDiagnosticContext<'a> {
    pub kind: CallSurfaceKind,
    pub callee_name: &'a str,
}

impl<'a> CallDiagnosticContext<'a> {
    pub(crate) fn function(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::Function,
            callee_name,
        }
    }

    pub(crate) fn struct_constructor(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::StructConstructor,
            callee_name,
        }
    }

    pub(crate) fn choice_constructor(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::ChoiceConstructor,
            callee_name,
        }
    }

    pub(crate) fn receiver_method(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::ReceiverMethod,
            callee_name,
        }
    }

    pub(crate) fn builtin_member(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::BuiltinMember,
            callee_name,
        }
    }

    pub(crate) fn host_function(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::HostFunction,
            callee_name,
        }
    }

    pub(crate) fn assertion(callee_name: &'a str) -> Self {
        Self {
            kind: CallSurfaceKind::Assertion,
            callee_name,
        }
    }

    fn callable_title(self) -> &'static str {
        match self.kind {
            CallSurfaceKind::Function => "Function",
            CallSurfaceKind::StructConstructor => "Struct constructor",
            CallSurfaceKind::ChoiceConstructor => "Choice constructor",
            CallSurfaceKind::ReceiverMethod => "Receiver method",
            CallSurfaceKind::BuiltinMember => "Builtin member",
            CallSurfaceKind::HostFunction => "Host function",
            CallSurfaceKind::Assertion => "Assertion",
        }
    }
}

pub(crate) enum ExpectedAccessMode {
    Shared,
    Mutable,
}

pub(crate) enum ExpectedParameterType {
    Known(TypeId),
    UnknownExternal,
}

pub(crate) struct ParameterExpectation {
    pub name: Option<StringId>,
    /// Canonical type for the parameter, or `UnknownExternal` when the external
    /// package does not expose a resolved frontend type.
    pub expected_type: ExpectedParameterType,
    pub access_mode: ExpectedAccessMode,
    pub requires_reactive_source: bool,
    pub default_value: Option<Expression>,
}

enum CallTypeValidation<'a> {
    Validate(&'a mut TypeCompatibilityCache),
    Skip,
}

pub(crate) struct CallArgumentResolutionContext<'a> {
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) compatibility_cache: &'a mut TypeCompatibilityCache,
}

struct CallArgumentPolicyContext<'a> {
    string_table: &'a mut StringTable,
    type_environment: &'a TypeEnvironment,
    type_validation: CallTypeValidation<'a>,
}

/// Builds one expectation per user-defined parameter declaration.
pub(crate) fn expectations_from_user_parameters(
    parameters: &[Declaration],
) -> Vec<ParameterExpectation> {
    parameters
        .iter()
        .map(|parameter| ParameterExpectation {
            name: parameter.id.name(),
            expected_type: ExpectedParameterType::Known(parameter.value.type_id),
            access_mode: if parameter.value.value_mode.is_mutable() {
                ExpectedAccessMode::Mutable
            } else {
                ExpectedAccessMode::Shared
            },
            requires_reactive_source: parameter.value.reactive_source.is_some(),
            default_value: match parameter.value.kind {
                ExpressionKind::NoValue => None,
                _ => {
                    let mut default_value = parameter.value.clone();
                    default_value.reactive_template = None;
                    Some(default_value)
                }
            },
        })
        .collect()
}

/// Maps a single external parameter into the frontend's `ParameterExpectation`.
///
/// WHAT: converts `ExternalSignatureType` and `ExternalAccessKind` into the canonical
///       `ExpectedParameterType` and `ExpectedAccessMode` used by call validation.
/// WHY: every external function, including package functions that operate on opaque handles,
///      uses the same argument validation path.
fn parameter_expectation_from_external(
    parameter: &ExternalParameter,
    type_environment: &mut TypeEnvironment,
) -> ParameterExpectation {
    let expected_type = match parameter
        .language_type
        .to_parameter_type_id(type_environment)
    {
        Some(type_id) => ExpectedParameterType::Known(type_id),
        None => ExpectedParameterType::UnknownExternal,
    };

    ParameterExpectation {
        name: None,
        expected_type,
        access_mode: match parameter.access_kind {
            ExternalAccessKind::Shared => ExpectedAccessMode::Shared,
            ExternalAccessKind::Mutable => ExpectedAccessMode::Mutable,
        },
        requires_reactive_source: false,
        default_value: None,
    }
}

pub(crate) fn expectations_from_host_function(
    function: &ExternalFunctionDef,
    type_environment: &mut TypeEnvironment,
) -> Vec<ParameterExpectation> {
    function
        .parameters
        .iter()
        .map(|parameter| parameter_expectation_from_external(parameter, type_environment))
        .collect()
}

pub(crate) fn expectations_from_constructor_fields(
    fields: &[ConstructorField],
) -> Vec<ParameterExpectation> {
    fields
        .iter()
        .map(|field| ParameterExpectation {
            name: field.name.name(),
            expected_type: ExpectedParameterType::Known(field.type_id),
            access_mode: match field.access_mode {
                ConstructorFieldAccessMode::Shared => ExpectedAccessMode::Shared,
                ConstructorFieldAccessMode::Mutable => ExpectedAccessMode::Mutable,
            },
            requires_reactive_source: false,
            default_value: field.default_value.clone(),
        })
        .collect()
}

pub(crate) fn expectations_from_receiver_method_signature(
    parameters_excluding_receiver: &[Declaration],
) -> Vec<ParameterExpectation> {
    expectations_from_user_parameters(parameters_excluding_receiver)
}

/// Resolves parser-owned call arguments through shared final validation.
///
/// WHAT: this is the normalization boundary for defaults, type compatibility and access policy
///      after `call_arguments` has selected each argument's declaration-order slot.
/// WHY: once one caller changes final argument policy, every other call-shaped consumer inherits
///      it from here without a second syntax-routing implementation.
pub(crate) fn resolve_call_arguments(
    diagnostics: CallDiagnosticContext<'_>,
    args: &[CallArgument],
    expectations: &[ParameterExpectation],
    location: SourceLocation,
    context: CallArgumentResolutionContext<'_>,
) -> Result<Vec<CallArgument>, CallValidationError> {
    resolve_call_arguments_with_type_policy(
        diagnostics,
        args,
        expectations,
        location,
        CallArgumentPolicyContext {
            string_table: context.string_table,
            type_environment: context.type_environment,
            type_validation: CallTypeValidation::Validate(context.compatibility_cache),
        },
    )
}

/// Resolves call arguments through shared shape/default/access rules without
/// running the ordinary final type-compatibility check.
///
/// WHAT: generic template validation first binds callee generic parameters to
/// caller generic `TypeId`s, then substitutes the call signature. At that point
/// exact `TypeId` equality is intentionally too strict for the pre-substitution
/// template parameters.
/// WHY: the parser owns named/positional routing; this owner consumes those retained slots for
/// arity, defaults and mutable-access rules while allowing generic-aware validation to supply its
/// own type evidence.
pub(crate) fn resolve_call_arguments_shape_and_access(
    diagnostics: CallDiagnosticContext<'_>,
    args: &[CallArgument],
    expectations: &[ParameterExpectation],
    location: SourceLocation,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> Result<Vec<CallArgument>, CallValidationError> {
    resolve_call_arguments_with_type_policy(
        diagnostics,
        args,
        expectations,
        location,
        CallArgumentPolicyContext {
            string_table,
            type_environment,
            type_validation: CallTypeValidation::Skip,
        },
    )
}

fn resolve_call_arguments_with_type_policy(
    diagnostics: CallDiagnosticContext<'_>,
    args: &[CallArgument],
    expectations: &[ParameterExpectation],
    location: SourceLocation,
    context: CallArgumentPolicyContext<'_>,
) -> Result<Vec<CallArgument>, CallValidationError> {
    let CallArgumentPolicyContext {
        string_table,
        type_environment,
        mut type_validation,
    } = context;

    // Validation flow order is intentionally fixed:
    // 1) consume parser-retained parameter slots,
    // 2) fill defaults,
    // 3) detect missing required parameters,
    // 4) validate types,
    // 5) validate access mode.
    let mut resolved = order_call_arguments_by_retained_slot(args, expectations.len())?;

    // ------------------------
    //  Fill default values
    // ------------------------
    for (slot, expectation) in expectations.iter().enumerate() {
        if resolved[slot].is_none() {
            if let Some(default_value) = &expectation.default_value {
                let defaulted = default_value.clone();
                debug_assert!(
                    type_environment.get(defaulted.type_id).is_some(),
                    "default argument expression carried orphan TypeId({}) not registered in the active TypeEnvironment",
                    defaulted.type_id.0,
                );
                resolved[slot] = Some(
                    CallArgument::positional(defaulted, CallAccessMode::Shared, location.clone())
                        .with_parameter_slot(ParameterSlot::new(slot)),
                );
            } else {
                return Err(CompilerDiagnostic::invalid_call_shape(
                    InvalidCallShapeReason::MissingArgument {
                        parameter_name: expectation.name,
                        parameter_index: slot,
                    },
                    Some(string_table.intern(diagnostics.callee_name)),
                    location.clone(),
                )
                .into());
            }
        }
    }

    // ------------------------
    //  Validate and coerce each argument
    // ------------------------
    let mut ordered = Vec::with_capacity(expectations.len());
    for (slot, expectation) in expectations.iter().enumerate() {
        let Some(argument) = resolved[slot].take() else {
            let message = format!(
                "Call argument resolution left required slot {} empty for {} '{}'",
                slot + 1,
                diagnostics.callable_title(),
                diagnostics.callee_name
            );
            return Err(CompilerError::compiler_error(message).into());
        };

        let passing_mode = classify_call_passing_mode(
            &diagnostics,
            &argument,
            expectation,
            slot,
            location.clone(),
            string_table,
        )?;

        if expectation.requires_reactive_source && !argument.value.is_reactive_source() {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::ReactiveSourceRequired {
                    parameter_name: expectation.name,
                    parameter_index: slot,
                },
                Some(string_table.intern(diagnostics.callee_name)),
                argument.location.clone(),
            )
            .into());
        }

        let mut normalized_argument = argument.with_passing_mode(passing_mode);
        if !expectation.requires_reactive_source {
            normalized_argument.value.clear_reactive_source();
        }

        let expected_type_id = match expectation.expected_type {
            ExpectedParameterType::Known(type_id) => type_id,
            ExpectedParameterType::UnknownExternal => {
                // Unknown external types skip compatibility checking.
                ordered.push(normalized_argument);
                continue;
            }
        };
        let actual_type_id = normalized_argument.value.type_id;

        if let CallTypeValidation::Validate(compatibility_cache) = &mut type_validation
            && !is_call_argument_type_compatible(
                expectation,
                actual_type_id,
                passing_mode,
                type_environment,
                compatibility_cache,
            )
        {
            let mismatch_context = match diagnostics.kind {
                CallSurfaceKind::StructConstructor | CallSurfaceKind::ChoiceConstructor => {
                    TypeMismatchContext::ConstructorArgument
                }
                CallSurfaceKind::ReceiverMethod | CallSurfaceKind::BuiltinMember => {
                    TypeMismatchContext::ReceiverArgument
                }
                CallSurfaceKind::Assertion => TypeMismatchContext::AssertionArgument,
                _ => TypeMismatchContext::FunctionArgument,
            };
            return Err(CompilerDiagnostic::type_mismatch(
                expected_type_id,
                actual_type_id,
                mismatch_context,
                normalized_argument.location.clone(),
            )
            .into());
        }

        normalized_argument.value = coerce_expression_to_declared_type(
            normalized_argument.value,
            expected_type_id,
            type_environment,
        );

        ordered.push(normalized_argument);
    }

    Ok(ordered)
}

fn classify_call_passing_mode(
    diagnostics: &CallDiagnosticContext<'_>,
    argument: &CallArgument,
    expectation: &ParameterExpectation,
    slot_index: usize,
    _location: SourceLocation,
    string_table: &mut StringTable,
) -> Result<CallPassingMode, CallValidationError> {
    let callee_name = Some(string_table.intern(diagnostics.callee_name));
    let source_state = classify_argument_source_state(&argument.value);

    match (argument.access_mode, &expectation.access_mode) {
        // Shared argument passed to a shared parameter: no restriction.
        (CallAccessMode::Shared, ExpectedAccessMode::Shared) => Ok(CallPassingMode::Shared),

        // Shared argument passed to a mutable parameter: fresh rvalues can be materialized for
        // mutable calls; existing places require the explicit `~` marker.
        (CallAccessMode::Shared, ExpectedAccessMode::Mutable) => match source_state {
            CallArgumentSourceState::Fresh => Ok(CallPassingMode::FreshMutableValue),
            // An immutable place without `~` is not fixed by adding a marker: the binding itself
            // must be declared mutable. Point at the value expression, since no `~` was authored,
            // so the compiler does not suggest a marker-only repair.
            CallArgumentSourceState::ImmutablePlace { binding_name } => {
                Err(CompilerDiagnostic::invalid_call_shape(
                    InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
                        parameter_name: expectation.name,
                        parameter_index: slot_index,
                        binding_name,
                    },
                    callee_name,
                    argument.location.clone(),
                )
                .into())
            }
            // A mutable place passed without `~` needs the explicit marker.
            CallArgumentSourceState::MutablePlace => Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::MutableAccessRequired {
                    parameter_name: expectation.name,
                    parameter_index: slot_index,
                },
                callee_name,
                argument.location.clone(),
            )
            .into()),
        },

        // Mutable argument passed to a shared parameter: never allowed.
        (CallAccessMode::Mutable, ExpectedAccessMode::Shared) => {
            Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::MutableAccessNotAllowed {
                    parameter_name: expectation.name,
                    parameter_index: slot_index,
                },
                callee_name,
                authored_marker_location(argument),
            )
            .into())
        }

        // Mutable argument passed to a mutable parameter: `~` accepts only an existing
        // mutable place.
        (CallAccessMode::Mutable, ExpectedAccessMode::Mutable) => match source_state {
            // Fresh rvalues, including explicit `copy source`, are not places here, so the
            // authored marker is the mistake.
            CallArgumentSourceState::Fresh => Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::MutableAccessOnNonPlace {
                    parameter_name: expectation.name,
                    parameter_index: slot_index,
                },
                callee_name,
                authored_marker_location(argument),
            )
            .into()),
            // The authored `~` is the call-site source the author wrote, so the primary label
            // stays on the marker rather than the immutable binding declaration.
            CallArgumentSourceState::ImmutablePlace { binding_name } => {
                Err(CompilerDiagnostic::invalid_call_shape(
                    InvalidCallShapeReason::MutableAccessOnImmutablePlace {
                        parameter_name: expectation.name,
                        parameter_index: slot_index,
                        binding_name,
                    },
                    callee_name,
                    authored_marker_location(argument),
                )
                .into())
            }
            CallArgumentSourceState::MutablePlace => Ok(CallPassingMode::MutablePlace),
        },
    }
}

/// Resolve the primary diagnostic location for an argument that authored a `~` marker.
///
/// WHAT: prefers the authored marker location, falling back to the value expression location.
/// WHY: when `~` was authored, the marker is the call-site source the author must change.
fn authored_marker_location(argument: &CallArgument) -> SourceLocation {
    argument
        .marker_location
        .clone()
        .unwrap_or_else(|| argument.location.clone())
}

fn is_call_argument_type_compatible(
    expectation: &ParameterExpectation,
    actual_type_id: TypeId,
    passing_mode: CallPassingMode,
    type_environment: &TypeEnvironment,
    compatibility_cache: &mut TypeCompatibilityCache,
) -> bool {
    let expected_type_id = match expectation.expected_type {
        ExpectedParameterType::Known(type_id) => type_id,
        ExpectedParameterType::UnknownExternal => {
            // Unknown expected type (e.g. external Handle/Void): skip compatibility check.
            return true;
        }
    };

    let compatibility_mode = if passing_mode == CallPassingMode::FreshMutableValue {
        TypeCompatibilityMode::FreshMutableRvalue
    } else {
        TypeCompatibilityMode::Standard
    };

    compatibility_cache.is_compatible(
        expected_type_id,
        actual_type_id,
        compatibility_mode,
        type_environment,
    )
}

/// Call-boundary classification of an argument expression's source state.
///
/// WHAT: distinguishes fresh rvalues from existing places, and for an existing place carries
///       its mutability and the simple root binding name when the place has a namable root.
/// WHY: mutable-access call validation needs all three facts together to choose the right
///      diagnostic, and computing them in one traversal keeps the call boundary's source-state
///      ownership in one place instead of three overlapping expression walks.
enum CallArgumentSourceState {
    /// A fresh rvalue: a literal, constructor, computed expression, or `copy` result. Fresh
    /// values satisfy a mutable parameter without a source `~` marker.
    Fresh,
    /// An existing immutable place. `binding_name` is the simple root binding name when one is
    /// namable, so immutable-place diagnostics can name the binding to declare mutable.
    ImmutablePlace { binding_name: Option<StringId> },
    /// An existing mutable place. Mutable places satisfy a mutable parameter with an authored
    /// `~` marker and need no binding name in current diagnostics.
    MutablePlace,
}

/// Classify an argument expression's call-boundary source state in a single traversal.
///
/// WHAT: walks the argument expression once to decide whether it is fresh or an existing place
///       and, for an existing place, its mutability and simple root binding name.
/// WHY: shared and authored-`~` mutable-parameter branches share one classification, so the
///      call boundary traverses the argument exactly once instead of once per fact.
fn classify_argument_source_state(expression: &Expression) -> CallArgumentSourceState {
    match &expression.kind {
        // A reference is the root: its mutability comes from the value mode, and its binding
        // name is the referenced path's simple name.
        ExpressionKind::Reference(path) => {
            let binding_name = path.name();
            if expression.value_mode.is_mutable() {
                CallArgumentSourceState::MutablePlace
            } else {
                CallArgumentSourceState::ImmutablePlace { binding_name }
            }
        }
        // A field access follows its base to the root place, inheriting the root's mutability
        // and binding name when the root is immutable.
        ExpressionKind::FieldAccess { base, .. } => classify_argument_source_state(base),
        // A single-operand runtime projection follows that operand.
        ExpressionKind::Runtime(rpn) if rpn.items.len() == 1 => match rpn.items.first() {
            Some(ExpressionRpnItem::Operand(inner)) => classify_argument_source_state(inner),
            _ => CallArgumentSourceState::Fresh,
        },
        // `copy` produces an independent value, so it is fresh at the call boundary even though
        // its operand is an existing place. Every other expression kind is a fresh rvalue.
        _ => CallArgumentSourceState::Fresh,
    }
}

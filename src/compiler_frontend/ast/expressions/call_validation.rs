//! Shared call-argument normalization and validation.
//!
//! WHAT: consumes parser-retained argument slots, fills defaults and enforces the shared rules for
//! type compatibility and explicit access mode.
//! WHY: function calls, struct constructors, receiver methods and builtin members all need the
//! same final argument policy even though they build different AST nodes afterward. Named and
//! positional syntax is owned by `call_arguments` and is not reconstructed here.

use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode, ParameterSlot,
    order_owned_call_arguments_by_retained_slot,
};
use crate::compiler_frontend::ast::expressions::constructor_views::{
    ConstructorField, ConstructorFieldAccessMode,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionConstructionFailure, ExpressionOperationView,
    ExpressionView, OwnedExpr,
};
use crate::compiler_frontend::ast::numeric_known_shape::KnownShapeId;
use rustc_hash::FxHashMap;

use crate::compiler_frontend::ast::statements::functions::{FunctionParameter, ParameterContract};
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCallShapeReason, TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::source::SourceSpan;

use crate::compiler_frontend::external_packages::{
    ExternalAccessKind, ExternalFunctionDef, ExternalParameter,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::type_coercion::compatibility::{
    TypeCompatibilityCache, TypeCompatibilityMode,
};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_declared_type;

pub(crate) enum CallValidationError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<InfrastructureErrors>),
}

impl From<CompilerDiagnostic> for CallValidationError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        CallValidationError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for CallValidationError {
    fn from(error: CompilerError) -> Self {
        CallValidationError::Infrastructure(error.into())
    }
}

impl From<Box<InfrastructureErrors>> for CallValidationError {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        Self::Infrastructure(errors)
    }
}

impl From<ExpressionConstructionFailure> for CallValidationError {
    fn from(error: ExpressionConstructionFailure) -> Self {
        match error {
            ExpressionConstructionFailure::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ExpressionConstructionFailure::Invariant(error) => error.into(),
        }
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

/// The callee spelling one call surface reports in its diagnostics.
///
/// WHAT: retains the source components of a callable's diagnostic name until a diagnostic needs
///       them: an interned declaration name, a path component whose absence has an established
///       spelling, a borrowed spelling, or a choice variant's two parts.
/// WHY: successful calls never render a callee name, so formatting a composite name and interning
///      it must happen on the error path only. Each surface's fallback spelling travels with the
///      descriptor instead of being resolved eagerly at the call site.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CallDiagnosticName<'a> {
    /// A name the source already interned: a declaration name or an existing path component.
    Interned(StringId),
    /// A path component that may be absent, with this surface's fallback spelling.
    PathComponent {
        component: Option<StringId>,
        fallback: &'static str,
    },
    /// A name owned by a borrowed spelling in this compilation.
    Borrowed(&'a str),
    /// `Choice::Variant`, formatted only when a diagnostic renders it.
    ChoiceVariant {
        choice_name: Option<StringId>,
        variant_name: StringId,
    },
}

impl<'a> CallDiagnosticName<'a> {
    pub(crate) fn diagnostic_callee_name(self, string_table: &mut StringTable) -> Option<StringId> {
        match self {
            Self::Interned(name) => Some(name),
            Self::PathComponent {
                component,
                fallback,
            } => Some(component.unwrap_or_else(|| string_table.intern(fallback))),
            Self::Borrowed(name) => Some(string_table.intern(name)),
            Self::ChoiceVariant {
                choice_name,
                variant_name,
            } => {
                let choice_name = choice_name
                    .map(|name| string_table.resolve(name))
                    .unwrap_or("<choice>");
                let variant_name = string_table.resolve(variant_name);
                Some(string_table.get_or_intern(format!("{choice_name}::{variant_name}")))
            }
        }
    }

    pub(crate) fn rendered_callee_name(self, string_table: &StringTable) -> String {
        match self {
            Self::Interned(name) => string_table.resolve(name).to_owned(),
            Self::PathComponent {
                component: Some(name),
                ..
            } => string_table.resolve(name).to_owned(),
            Self::PathComponent {
                component: None,
                fallback,
            } => String::from(fallback),
            Self::Borrowed(name) => String::from(name),
            Self::ChoiceVariant {
                choice_name,
                variant_name,
            } => {
                let choice_name = choice_name
                    .map(|name| string_table.resolve(name))
                    .unwrap_or("<choice>");
                let variant_name = string_table.resolve(variant_name);
                format!("{choice_name}::{variant_name}")
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CallDiagnosticContext<'a> {
    pub kind: CallSurfaceKind,
    callee_name: CallDiagnosticName<'a>,
}

impl<'a> CallDiagnosticContext<'a> {
    pub(crate) fn function(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::Function,
            callee_name,
        }
    }

    pub(crate) fn struct_constructor(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::StructConstructor,
            callee_name,
        }
    }

    pub(crate) fn choice_constructor(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::ChoiceConstructor,
            callee_name,
        }
    }

    pub(crate) fn receiver_method(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::ReceiverMethod,
            callee_name,
        }
    }

    pub(crate) fn builtin_member(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::BuiltinMember,
            callee_name,
        }
    }

    pub(crate) fn host_function(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::HostFunction,
            callee_name,
        }
    }

    pub(crate) fn assertion(callee_name: CallDiagnosticName<'a>) -> Self {
        Self {
            kind: CallSurfaceKind::Assertion,
            callee_name,
        }
    }

    pub(crate) fn diagnostic_callee_name(self, string_table: &mut StringTable) -> Option<StringId> {
        self.callee_name.diagnostic_callee_name(string_table)
    }

    pub(crate) fn rendered_callee_name(self, string_table: &StringTable) -> String {
        self.callee_name.rendered_callee_name(string_table)
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

#[derive(Debug, Clone)]
pub(crate) enum ExpectedAccessMode {
    Shared,
    Mutable,
}

#[derive(Debug, Clone)]
pub(crate) enum ExpectedParameterType {
    Known(TypeId),
    /// Construction-only numeric slot shape under a retained collection or map descriptor.
    /// WHY: its canonical TypeId is decided at body-root completion, so it is materialised to
    /// `Known` before any validation. It never reaches a completed call, signature, or HIR.
    ConstructionShape(KnownShapeId),
    /// Generic template parameter consumed by both numeric retention and call inference.
    /// Completed compatibility checks use the instantiated parameter's concrete type.
    GenericParameter(TypeId),
    UnknownExternal,
}

/// Default content a call fills for an omitted parameter.
///
/// WHAT: `Folded` is canonical constant content. `Authored` is a struct-field default expression
/// that the strict struct pass has not folded yet.
/// WHY: a struct default can name a module constant that is not yet published when the struct
/// shell is built. Whether a default exists is the declaration's fact; fold state only chooses the
/// representation.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ParameterDefault {
    /// Canonical folded default; each omitting call receives a constant occurrence of it.
    Folded(ConstValueId),
    /// Authored default expression. Each omitting call shares it; the enclosing constant folds it
    /// in its own context.
    Authored(ExprId),
}

#[derive(Debug, Clone)]
pub(crate) struct ParameterExpectation {
    pub name: Option<StringId>,
    /// Canonical type for the parameter, or `UnknownExternal` when the external
    /// package does not expose a resolved frontend type.
    pub expected_type: ExpectedParameterType,
    pub access_mode: ExpectedAccessMode,
    pub default_value: Option<ParameterDefault>,
    pub contract: ParameterContract,
}

enum CallTypeValidation<'a> {
    Validate(&'a mut TypeCompatibilityCache),
    Skip,
}

pub(crate) struct CallArgumentResolutionContext<'a> {
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) compatibility_cache: &'a mut TypeCompatibilityCache,
    pub(crate) path_fork: &'a PathInternerFork,
}

struct CallArgumentPolicyContext<'a> {
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    string_table: &'a mut StringTable,
    type_environment: &'a TypeEnvironment,
    type_validation: CallTypeValidation<'a>,
    path_fork: &'a PathInternerFork,
}

/// Builds one expectation per user-defined parameter declaration.
pub(crate) fn expectations_from_user_parameters(
    parameters: &[FunctionParameter],
    path_fork: &PathInternerFork,
) -> Vec<ParameterExpectation> {
    parameters
        .iter()
        .map(|parameter| ParameterExpectation {
            name: path_fork.component(parameter.id),
            expected_type: ExpectedParameterType::Known(parameter.type_id),
            access_mode: if parameter.value_mode.is_mutable() {
                ExpectedAccessMode::Mutable
            } else {
                ExpectedAccessMode::Shared
            },
            default_value: parameter.default.map(ParameterDefault::Folded),
            contract: parameter.contract,
        })
        .collect()
}

/// Builds one expectation per generic template parameter declaration.
///
/// WHAT: a declared type that mentions a generic parameter becomes a `GenericParameter`
///       expectation whose concrete type inference chooses later; other declared types stay `Known`.
/// WHY: parse-side numeric retention must never treat the template's bare `T` as a concrete slot type.
pub(crate) fn generic_template_expectations(
    parameters: &[FunctionParameter],
    path_fork: &PathInternerFork,
    type_environment: &TypeEnvironment,
) -> Vec<ParameterExpectation> {
    let mut expectations = expectations_from_user_parameters(parameters, path_fork);
    for (expectation, parameter) in expectations.iter_mut().zip(parameters) {
        if type_environment.contains_generic_parameter(parameter.type_id) {
            expectation.expected_type = ExpectedParameterType::GenericParameter(parameter.type_id);
        }
    }
    expectations
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
        default_value: None,
        contract: ParameterContract::Value,
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
    path_fork: &PathInternerFork,
) -> Vec<ParameterExpectation> {
    fields
        .iter()
        .map(|field| parameter_expectation_from_constructor_field(field, path_fork, field.type_id))
        .collect()
}

/// Builds post-inference expectations directly from declared constructor views.
///
/// WHY: only the expected TypeIds change after generic inference; names, access and defaults still
/// belong to the original field descriptors and need no substituted field-vector copy.
pub(crate) fn expectations_from_constructor_fields_with_substitution(
    fields: &[ConstructorField],
    path_fork: &PathInternerFork,
    type_environment: &mut TypeEnvironment,
    replacements: &FxHashMap<GenericParameterId, TypeId>,
) -> Vec<ParameterExpectation> {
    fields
        .iter()
        .map(|field| {
            let type_id = type_environment.substitute_type_id(field.type_id, replacements);
            parameter_expectation_from_constructor_field(field, path_fork, type_id)
        })
        .collect()
}

fn parameter_expectation_from_constructor_field(
    field: &ConstructorField,
    path_fork: &PathInternerFork,
    type_id: TypeId,
) -> ParameterExpectation {
    #[cfg(feature = "benchmark_counters")]
    if field.default_value.is_some() {
        increment_frontend_counter(FrontendCounter::CensusParameterDefaultViewClones);
    }

    ParameterExpectation {
        name: path_fork.component(field.name),
        expected_type: ExpectedParameterType::Known(type_id),
        access_mode: match field.access_mode {
            ConstructorFieldAccessMode::Shared => ExpectedAccessMode::Shared,
            ConstructorFieldAccessMode::Mutable => ExpectedAccessMode::Mutable,
        },
        default_value: field.default_value,
        contract: ParameterContract::Value,
    }
}

pub(crate) fn expectations_from_receiver_method_signature(
    parameters_excluding_receiver: &[FunctionParameter],
    path_fork: &PathInternerFork,
) -> Vec<ParameterExpectation> {
    expectations_from_user_parameters(parameters_excluding_receiver, path_fork)
}

/// Resolves parser-owned call arguments through shared final validation.
///
/// WHAT: this is the normalization boundary for defaults, type compatibility and access policy
///      after `call_arguments` has selected each argument's declaration-order slot.
/// WHY: one owned vector lets every call surface share this final policy while moving arguments
///      into parameter order instead of cloning them.
pub(crate) fn resolve_call_arguments(
    diagnostics: CallDiagnosticContext<'_>,
    arguments: Vec<CallArgument<OwnedExpr>>,
    expectations: &[ParameterExpectation],
    span: Option<SourceSpan>,
    context: CallArgumentResolutionContext<'_>,
) -> Result<Vec<CallArgument<OwnedExpr>>, CallValidationError> {
    resolve_call_arguments_with_type_policy(
        diagnostics,
        arguments,
        expectations,
        span,
        CallArgumentPolicyContext {
            expression_builder: context.expression_builder,
            const_values: context.const_values,
            string_table: context.string_table,
            type_environment: context.type_environment,
            type_validation: CallTypeValidation::Validate(context.compatibility_cache),
            path_fork: context.path_fork,
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
/// arity, defaults and mutable-access rules while allowing generic-aware validation to supply
/// its own type evidence.
#[allow(
    clippy::too_many_arguments,
    reason = "call shape validation keeps diagnostics, owned arguments, expectations, span, expression builder, const store, string table, type environment, and path fork as separate inputs"
)]
pub(crate) fn resolve_call_arguments_shape_and_access(
    diagnostics: CallDiagnosticContext<'_>,
    arguments: Vec<CallArgument<OwnedExpr>>,
    expectations: &[ParameterExpectation],
    span: Option<SourceSpan>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> Result<Vec<CallArgument<OwnedExpr>>, CallValidationError> {
    resolve_call_arguments_with_type_policy(
        diagnostics,
        arguments,
        expectations,
        span,
        CallArgumentPolicyContext {
            expression_builder,
            const_values,
            string_table,
            type_environment,
            type_validation: CallTypeValidation::Skip,
            path_fork,
        },
    )
}

fn call_value_receiver(kind: CallSurfaceKind) -> ValueReceiverContext {
    match kind {
        CallSurfaceKind::StructConstructor | CallSurfaceKind::ChoiceConstructor => {
            ValueReceiverContext::ConstructorArgument
        }
        CallSurfaceKind::Assertion => ValueReceiverContext::AssertionArgument,
        _ => ValueReceiverContext::FunctionArgument,
    }
}

fn resolve_call_arguments_with_type_policy(
    diagnostics: CallDiagnosticContext<'_>,
    args: Vec<CallArgument<OwnedExpr>>,
    expectations: &[ParameterExpectation],
    span: Option<SourceSpan>,
    context: CallArgumentPolicyContext<'_>,
) -> Result<Vec<CallArgument<OwnedExpr>>, CallValidationError> {
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(match diagnostics.kind {
        CallSurfaceKind::Function => FrontendCounter::CensusCallFunctionEntries,
        CallSurfaceKind::StructConstructor => FrontendCounter::CensusCallStructConstructorEntries,
        CallSurfaceKind::ChoiceConstructor => FrontendCounter::CensusCallChoiceConstructorEntries,
        CallSurfaceKind::ReceiverMethod => FrontendCounter::CensusCallReceiverMethodEntries,
        CallSurfaceKind::BuiltinMember => FrontendCounter::CensusCallBuiltinMemberEntries,
        CallSurfaceKind::HostFunction => FrontendCounter::CensusCallHostFunctionEntries,
        CallSurfaceKind::Assertion => FrontendCounter::CensusCallAssertionEntries,
    });
    let CallArgumentPolicyContext {
        expression_builder,
        const_values,
        string_table,
        type_environment,
        mut type_validation,
        path_fork,
    } = context;

    // Validation flow order is intentionally fixed:
    // 1) consume parser-retained parameter slots,
    // 2) fill defaults and detect missing required parameters,
    // 3) classify access/passing mode, which feeds compatibility checks,
    // 4) validate and coerce known types while retaining opaque external values.
    let mut resolved = order_owned_call_arguments_by_retained_slot(args, expectations.len())?;

    // ------------------------
    //  Fill default values
    // ------------------------
    for (slot, expectation) in expectations.iter().enumerate() {
        if resolved[slot].is_none() {
            if let Some(default_value) = expectation.default_value {
                #[cfg(feature = "benchmark_counters")]
                increment_frontend_counter(FrontendCounter::CensusDefaultApplicationClones);
                let defaulted = match default_value {
                    ParameterDefault::Folded(default_value) => {
                        // Canonical parameter defaults are content metadata; occurrence facts are projected
                        // from that content onto the call site.
                        let metadata = const_values
                            .metadata(default_value)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "default argument constant ID outside owner",
                                )
                            })?
                            .clone();
                        expression_builder.constant(
                            default_value,
                            metadata.type_id,
                            span,
                            metadata.value_mode,
                        )?
                    }
                    // Authored struct-field defaults are spliced as the authored expression. Only the
                    // strict struct pass folds them and rejects any that stay unfolded, so a runtime
                    // call never sees this variant.
                    ParameterDefault::Authored(expression) => expression_builder.own(expression),
                };
                expression_builder
                    .view_owned(defaulted)?
                    .require_single(call_value_receiver(diagnostics.kind))?;
                resolved[slot] = Some(
                    CallArgument::positional(defaulted, CallAccessMode::Shared, span)
                        .with_parameter_slot(ParameterSlot::new(slot)),
                );
                #[cfg(feature = "benchmark_counters")]
                increment_frontend_counter(match diagnostics.kind {
                    CallSurfaceKind::Function => FrontendCounter::CensusCallFunctionDefaultFills,
                    CallSurfaceKind::StructConstructor => {
                        FrontendCounter::CensusCallStructConstructorDefaultFills
                    }
                    CallSurfaceKind::ChoiceConstructor => {
                        FrontendCounter::CensusCallChoiceConstructorDefaultFills
                    }
                    CallSurfaceKind::ReceiverMethod => {
                        FrontendCounter::CensusCallReceiverMethodDefaultFills
                    }
                    CallSurfaceKind::BuiltinMember => {
                        FrontendCounter::CensusCallBuiltinMemberDefaultFills
                    }
                    CallSurfaceKind::HostFunction => {
                        FrontendCounter::CensusCallHostFunctionDefaultFills
                    }
                    CallSurfaceKind::Assertion => FrontendCounter::CensusCallAssertionDefaultFills,
                });
            } else {
                return Err(CompilerDiagnostic::invalid_call_shape(
                    InvalidCallShapeReason::MissingArgument {
                        parameter_name: expectation.name,
                        parameter_index: slot,
                    },
                    diagnostics.diagnostic_callee_name(string_table),
                    span,
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
                diagnostics.rendered_callee_name(string_table)
            );
            return Err(CompilerError::compiler_error(message).into());
        };
        let actual_type_id = expression_builder
            .view_owned(argument.value)?
            .require_single(call_value_receiver(diagnostics.kind))?;

        let passing_mode = classify_call_passing_mode(
            &diagnostics,
            &argument,
            expression_builder,
            expectation,
            slot,
            string_table,
            path_fork,
        )?;

        let mut normalized_argument = argument.with_passing_mode(passing_mode);

        let expected_type_id = match expectation.expected_type {
            ExpectedParameterType::Known(type_id) => type_id,
            ExpectedParameterType::ConstructionShape(_) => {
                // WHY: construction-only shapes are materialised at body-root completion before
                // completed call validation; reaching here is a compiler ordering bug.
                return Err(CompilerError::compiler_error(
                    "Construction-only numeric shape reached completed call validation.",
                )
                .into());
            }
            ExpectedParameterType::GenericParameter(_) => {
                // WHY: generic inference and parse-side retention consume template expectations;
                // completed calls validate against the instantiated signature, never the template.
                return Err(CompilerError::compiler_error(
                    "Generic parameter expectation reached completed call validation.",
                )
                .into());
            }
            ExpectedParameterType::UnknownExternal => {
                // Unknown external types skip compatibility checking.
                ordered.push(normalized_argument);
                continue;
            }
        };

        let types_match = match (&mut type_validation, expectation.contract) {
            (CallTypeValidation::Validate(_), ParameterContract::Wire) => {
                actual_type_id == expected_type_id
            }
            (CallTypeValidation::Validate(compatibility_cache), _) => {
                is_call_argument_type_compatible(
                    expected_type_id,
                    actual_type_id,
                    passing_mode,
                    type_environment,
                    compatibility_cache,
                )
            }
            (CallTypeValidation::Skip, _) => true,
        };
        if !types_match {
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
                normalized_argument.span,
            )
            .into());
        }

        normalized_argument.value = coerce_expression_to_declared_type(
            normalized_argument.value,
            expected_type_id,
            type_environment,
            expression_builder,
        )?;

        ordered.push(normalized_argument);
    }

    Ok(ordered)
}

fn classify_call_passing_mode(
    diagnostics: &CallDiagnosticContext<'_>,
    argument: &CallArgument<OwnedExpr>,
    expression_builder: &AstExpressionBuilder,
    expectation: &ParameterExpectation,
    slot_index: usize,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<CallPassingMode, CallValidationError> {
    let source_state =
        classify_argument_source_state(expression_builder.view_owned(argument.value)?, path_fork);
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
                    diagnostics.diagnostic_callee_name(string_table),
                    argument.span,
                )
                .into())
            }
            // A mutable place passed without `~` needs the explicit marker.
            CallArgumentSourceState::MutablePlace => Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::MutableAccessRequired {
                    parameter_name: expectation.name,
                    parameter_index: slot_index,
                },
                diagnostics.diagnostic_callee_name(string_table),
                argument.span,
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
                diagnostics.diagnostic_callee_name(string_table),
                authored_marker_span(argument),
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
                diagnostics.diagnostic_callee_name(string_table),
                authored_marker_span(argument),
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
                    diagnostics.diagnostic_callee_name(string_table),
                    authored_marker_span(argument),
                )
                .into())
            }
            CallArgumentSourceState::MutablePlace => Ok(CallPassingMode::MutablePlace),
        },
    }
}

/// Resolve the primary diagnostic span for an argument that authored a `~` marker.
///
/// WHAT: prefers the authored marker span, falling back to the value expression span.
/// WHY: when `~` was authored, the marker is the call-site source the author must change.
fn authored_marker_span(argument: &CallArgument<OwnedExpr>) -> Option<SourceSpan> {
    argument.marker_span.or(argument.span)
}

fn is_call_argument_type_compatible(
    expected_type_id: TypeId,
    actual_type_id: TypeId,
    passing_mode: CallPassingMode,
    type_environment: &TypeEnvironment,
    compatibility_cache: &mut TypeCompatibilityCache,
) -> bool {
    // Known-type cache-call attempts include exact identity. Unknown external
    // slots never reach here; this is deliberately not the cache lookup/miss metric.
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusCallKnownTypeCompatibilityAttempts);

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
fn classify_argument_source_state(
    expression: ExpressionView<'_>,
    path_fork: &PathInternerFork,
) -> CallArgumentSourceState {
    match expression.operation() {
        ExpressionOperationView::Reference(path) => {
            let binding_name = path_fork.component(path);
            if expression.value_mode().is_mutable() {
                CallArgumentSourceState::MutablePlace
            } else {
                CallArgumentSourceState::ImmutablePlace { binding_name }
            }
        }
        ExpressionOperationView::FieldAccess { base, .. } => {
            classify_argument_source_state(base, path_fork)
        }
        _ => CallArgumentSourceState::Fresh,
    }
}

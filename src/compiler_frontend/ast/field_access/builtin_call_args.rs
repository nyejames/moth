//! Shared argument expectations and validation for builtin receiver members.
//!
//! WHAT: adapts a builtin operation's fixed parameter types to call-validation expectations and
//!       runs the shared call-argument resolution with the builtin-member diagnostic surface.
//! WHY: collection, map and error builtins share positional-only routing, arity, access-mode and
//!      type-compatibility rules, so the completed lane and the retained lane both validate
//!      through this one owner instead of restating the contract.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, CallDiagnosticName, ExpectedAccessMode,
    ExpectedParameterType, ParameterExpectation, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, OwnedExpr,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeId, materialise_known_shape,
};
use crate::compiler_frontend::ast::statements::functions::ParameterContract;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::member_rules::BuiltinArgumentSlots;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Adapts the shared allocation-free member rule to the AST argument owner's shape list.
pub(super) fn builtin_argument_shapes(
    slots: BuiltinArgumentSlots<KnownShapeId>,
) -> Vec<KnownShapeId> {
    match slots {
        BuiltinArgumentSlots::Zero => Vec::new(),
        BuiltinArgumentSlots::One(value) => vec![value],
        BuiltinArgumentSlots::Two(first, second) => vec![first, second],
    }
}

/// The concrete type a construction shape already names, when it is a concrete leaf.
pub(crate) fn concrete_shape_type(
    scope_context: &ScopeContext,
    shape: KnownShapeId,
) -> Option<TypeId> {
    match scope_context.numeric_state().shape(shape) {
        KnownShape::Concrete { type_id, .. } => Some(type_id),
        KnownShape::Numeric(_)
        | KnownShape::Collection { .. }
        | KnownShape::Map { .. }
        | KnownShape::Optional { .. }
        | KnownShape::DeclaredSubstitution { .. } => None,
    }
}

/// Interns one concrete type as a construction shape so a retained chain can carry it.
pub(super) fn intern_concrete_shape(
    scope_context: &ScopeContext,
    type_id: TypeId,
) -> Result<KnownShapeId, ExpressionParseError> {
    scope_context
        .numeric_state_mut()
        .intern_shape(KnownShape::Concrete {
            type_id,
            span: None,
        })
}

/// Materialises a construction shape into its canonical type once every leaf is concrete.
pub(super) fn materialise_builtin_shape(
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    shape: KnownShapeId,
) -> Result<TypeId, ExpressionParseError> {
    let mut state = scope_context.numeric_state_mut();
    materialise_known_shape(
        &mut state,
        type_interner.environment_mut_for_derived_types(),
        shape,
    )
}

/// Materialises only success slots; the call retains its error channel separately.
pub(super) fn builtin_result_type_ids(
    success_shape: Option<KnownShapeId>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
) -> Result<Vec<TypeId>, ExpressionParseError> {
    let Some(success_shape) = success_shape else {
        return Ok(Vec::new());
    };
    let success_type_id = materialise_builtin_shape(scope_context, type_interner, success_shape)?;
    Ok(vec![success_type_id])
}

/// The success slot as concrete types, when the shape is already concrete.
///
/// WHAT: returns `None` when the success shape is still a construction-only numeric or collection
///       shape, so the fragment handling site defers its success list to body-root completion.
/// WHY: a handling site validates its success list against concrete types only; an unresolved
///      numeric element is decided at completion, never defaulted here.
pub(super) fn builtin_success_type_ids(
    scope_context: &ScopeContext,
    success_shape: Option<KnownShapeId>,
) -> Option<Vec<TypeId>> {
    let Some(success_shape) = success_shape else {
        return Some(Vec::new());
    };
    concrete_shape_type(scope_context, success_shape).map(|type_id| vec![type_id])
}

/// Builds one builtin operation's argument expectations from its parameter slots.
pub(super) fn builtin_argument_expectations(
    slots: Vec<ExpectedParameterType>,
) -> Vec<ParameterExpectation> {
    slots
        .into_iter()
        .map(|expected_type| ParameterExpectation {
            name: None,
            expected_type,
            access_mode: ExpectedAccessMode::Shared,
            default_value: None,
            contract: ParameterContract::Value,
        })
        .collect()
}

/// Describes one builtin operation's argument slots for parse-time expectations.
///
/// WHAT: a concrete slot stays a canonical type; a numeric or collection slot stays a construction
///       shape.
/// WHY: a retained chain must not force an open numeric leaf before body-root completion.
pub(super) fn builtin_argument_slots(
    scope_context: &ScopeContext,
    shapes: &[KnownShapeId],
) -> Vec<ExpectedParameterType> {
    let state = scope_context.numeric_state();
    shapes
        .iter()
        .map(|shape| match state.shape(*shape) {
            KnownShape::Concrete { type_id, .. } => ExpectedParameterType::Known(type_id),
            KnownShape::Numeric(_)
            | KnownShape::Collection { .. }
            | KnownShape::Map { .. }
            | KnownShape::Optional { .. }
            | KnownShape::DeclaredSubstitution { .. } => {
                ExpectedParameterType::ConstructionShape(*shape)
            }
        })
        .collect()
}

/// Whether any builtin expectation still needs materialising before validation.
///
/// WHAT: reports whether the list holds an open numeric/collection slot or an unbound generic
///       parameter, which the materialiser must resolve or reject before validation runs.
/// WHY: a concrete expectation list is already the canonical validation form, so materialising it
///      again would rebuild identical slots and expectations for every builtin call.
fn has_open_builtin_slots(expectations: &[ParameterExpectation]) -> bool {
    expectations.iter().any(|expectation| {
        matches!(
            &expectation.expected_type,
            ExpectedParameterType::ConstructionShape(_)
                | ExpectedParameterType::GenericParameter(_)
        )
    })
}

/// Materialises every construction-only builtin slot before argument validation.
///
/// WHAT: a numeric or collection slot becomes its canonical type through the same materialiser the
///       result slots use; concrete and external slots pass through unchanged.
/// WHY: validation compares canonical types only, so an open slot must never reach completed call
///      validation. A ready argument registers its exact receiver before this point, so a
///      compatible argument materialises to its own type; only an incompatible argument is judged
///      against the class default.
fn materialised_builtin_expectations(
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expectations: &[ParameterExpectation],
) -> Result<Vec<ParameterExpectation>, ExpressionParseError> {
    let mut slots = Vec::with_capacity(expectations.len());
    for expectation in expectations {
        slots.push(match &expectation.expected_type {
            ExpectedParameterType::Known(type_id) => ExpectedParameterType::Known(*type_id),
            ExpectedParameterType::UnknownExternal => ExpectedParameterType::UnknownExternal,
            ExpectedParameterType::GenericParameter(_) => crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "a builtin member expectation carried a generic template parameter",
            ),
            ExpectedParameterType::ConstructionShape(shape) => ExpectedParameterType::Known(
                materialise_builtin_shape(context, type_interner, *shape)?,
            ),
        });
    }
    Ok(builtin_argument_expectations(slots))
}

/// The authored diagnostic identity of one builtin member call.
///
/// WHAT: the member's canonical source name and the span of its authored name.
/// WHY: a builtin member has no callable signature, so argument diagnostics name it from this
///      record alone. The retained lane builds the same record, so the diagnostic reads the same
///      whether the arguments resolve immediately or after a numeric origin completes.
#[derive(Clone, Copy)]
pub(super) struct BuiltinCallSite<'a> {
    /// The operation's canonical source spelling, as the diagnostic renders it.
    pub(super) member_name: &'a str,
    /// The source span of the authored member name, when the call has one.
    pub(super) member_span: Option<SourceSpan>,
}

/// Validate one builtin member's parsed arguments against its operation's expectations.
///
/// WHAT: runs the shared argument resolution — arity, positional-only routing, defaults, access
///       modes and final type compatibility — with the builtin-member diagnostic surface and
///       returns the resolved arguments the builtin constructor consumes.
/// WHY: a builtin member has no callable signature, so its fixed parameter expectations are the
///      whole contract; the completed lane validates here before construction and the retained lane
///      resumes the exact same owner once its arguments resolve, so no validation lane is skipped.
#[allow(
    clippy::too_many_arguments,
    reason = "builtin argument resolution keeps the call site, argument input, expectations, scope, type interner, expression builder, const store, string table, and path fork as separate inputs"
)]
pub(super) fn resolve_builtin_member_arguments(
    call_site: BuiltinCallSite<'_>,
    arguments: Vec<CallArgument<OwnedExpr>>,
    expectations: &[ParameterExpectation],
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<Vec<CallArgument<OwnedExpr>>, ExpressionParseError> {
    let materialised_expectations;
    let expectations = if has_open_builtin_slots(expectations) {
        materialised_expectations =
            materialised_builtin_expectations(context, type_interner, expectations)?;
        materialised_expectations.as_slice()
    } else {
        expectations
    };
    let type_check_context = type_interner.type_check_context();

    Ok(resolve_call_arguments(
        CallDiagnosticContext::builtin_member(CallDiagnosticName::Borrowed(call_site.member_name)),
        arguments,
        expectations,
        call_site.member_span,
        CallArgumentResolutionContext {
            expression_builder,
            const_values,
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork,
        },
    )?)
}

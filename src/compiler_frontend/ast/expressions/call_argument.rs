//! Canonical AST call-argument metadata.
//!
//! WHAT: carries per-argument metadata for call-shaped AST nodes.
//! WHY: plain expression vectors cannot represent named-target routing or explicit call-site
//! mutable access markers, and mutable-place vs fresh-rvalue passing semantics.

use crate::compiler_frontend::ast::expressions::expression_store::ExprId;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Stable declaration-order slot retained from call-argument parsing.
///
/// WHAT: identifies the parameter selected before the argument expression was parsed.
/// WHY: later validation must consume this fact rather than rerun named/positional routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParameterSlot(usize);

impl ParameterSlot {
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallAccessMode {
    Shared,
    Mutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPassingMode {
    Shared,
    MutablePlace,
    FreshMutableValue,
}

#[derive(Debug, Clone)]
pub(crate) struct CallArgument<E = ExprId> {
    /// Raw in retained records and owner-checked while construction is in flight.
    pub value: E,

    /// For named arguments, the interned parameter name this argument routes to.
    pub target_param: Option<StringId>,

    /// Parse-time access marker from the argument syntax.
    ///
    /// WHAT: preserves whether the user wrote `~` in source.
    /// WHY: call-resolution diagnostics still depend on explicit marker intent.
    pub access_mode: CallAccessMode,

    /// Post-validation passing classification used by lowering/analysis.
    ///
    /// WHAT: distinguishes mutable-place calls from mutable fresh-rvalue calls.
    /// WHY: HIR lowering needs this explicit distinction to synthesize hidden locals only when
    /// needed, without rediscovering policy from expression shape.
    pub passing_mode: CallPassingMode,

    /// Exact source span of the argument expression.
    pub span: Option<SourceSpan>,

    /// For named arguments, the exact source span of the parameter name token.
    pub target_span: Option<SourceSpan>,

    /// Optional exact source span of the authored `~` mutable-access marker.
    ///
    /// WHAT: preserves the span of the `~` token when the author wrote it.
    /// WHY: mutable-access diagnostics point at the marker when it is the real mistake, and at
    /// the value expression when the marker is absent, so the primary label stays on the
    /// authored source that the author must change.
    pub marker_span: Option<SourceSpan>,

    /// Exact source span of the comma immediately following this argument, when present.
    ///
    /// WHAT: retains the shared list owner's separator span for receiving-boundary arity
    /// diagnostics without rescanning the completed argument list.
    /// WHY: directive adapters can point at an offending separator while the shared parser
    /// remains the sole delimiter owner.
    pub(crate) following_separator_span: Option<SourceSpan>,

    /// Parameter slot selected by the shared parser before this value was parsed.
    ///
    /// WHAT: retains the parser's named/positional routing decision through AST validation.
    /// WHY: final validation must not reconstruct call syntax or produce a second routing policy.
    pub(crate) parameter_slot: Option<ParameterSlot>,

    /// Future input carried by a forwarded Route argument for immediate generic inference.
    ///
    /// WHAT: retains the input side of a Route reference, whose ordinary expression type only
    ///       carries the output choice.
    /// WHY: a nested generic call can infer its route input from the enclosing Route contract
    ///      without inspecting either callable body.
    pub(crate) route_future_input_type: Option<TypeId>,
}

impl<E> CallArgument<E> {
    /// Map the parse-time access marker to its default passing-mode classification.
    ///
    /// WHAT: `Shared` stays shared; `Mutable` starts as `MutablePlace` pending validation.
    /// WHY: validation may later upgrade or downgrade the classification via `with_passing_mode`.
    fn passing_mode_from_access_mode(access_mode: CallAccessMode) -> CallPassingMode {
        match access_mode {
            CallAccessMode::Shared => CallPassingMode::Shared,
            // Parse-time `~` is provisional; validation confirms this is actually a mutable place.
            CallAccessMode::Mutable => CallPassingMode::MutablePlace,
        }
    }

    /// Build a positional call argument with the default passing mode derived from `access_mode`.
    pub fn positional(value: E, access_mode: CallAccessMode, span: Option<SourceSpan>) -> Self {
        Self {
            value,
            target_param: None,
            access_mode,
            passing_mode: Self::passing_mode_from_access_mode(access_mode),
            span,
            target_span: None,
            marker_span: None,
            following_separator_span: None,
            parameter_slot: None,
            route_future_input_type: None,
        }
    }
    /// Build a named call argument with the default passing mode derived from `access_mode`.
    pub fn named(
        value: E,
        name: StringId,
        access_mode: CallAccessMode,
        span: Option<SourceSpan>,
        target_span: Option<SourceSpan>,
    ) -> Self {
        Self {
            value,
            target_param: Some(name),
            access_mode,
            passing_mode: Self::passing_mode_from_access_mode(access_mode),
            span,
            target_span,
            marker_span: None,
            following_separator_span: None,
            parameter_slot: None,
            route_future_input_type: None,
        }
    }

    /// Override the passing mode after validation has refined the provisional parse-time classification.
    pub fn with_passing_mode(mut self, passing_mode: CallPassingMode) -> Self {
        self.passing_mode = passing_mode;
        self
    }

    /// Attach the exact span of the authored `~` mutable-access marker.
    ///
    /// WHAT: only the parse owner populates this, because only it sees the `~` token.
    /// WHY: synthetic/test constructors that do not parse authored tokens leave it `None`.
    pub fn with_marker_span(mut self, marker_span: Option<SourceSpan>) -> Self {
        self.marker_span = marker_span;
        self
    }

    /// Retain the exact comma span following this argument.
    pub(crate) fn with_following_separator_span(
        mut self,
        following_separator_span: Option<SourceSpan>,
    ) -> Self {
        self.following_separator_span = following_separator_span;
        self
    }

    /// Retain the parser-selected declaration-order parameter slot.
    pub(crate) fn with_parameter_slot(mut self, parameter_slot: ParameterSlot) -> Self {
        self.parameter_slot = Some(parameter_slot);
        self
    }

    /// Retain the future input selected by Route forwarding.
    ///
    /// WHY: the expression's ordinary type carries only the output choice, so generic inference
    ///      needs this parse-time contract fact to recover the forwarded input.
    pub(crate) fn with_route_future_input_type(mut self, input_type: TypeId) -> Self {
        self.route_future_input_type = Some(input_type);
        self
    }

    /// Change only the expression edge when entering or leaving construction.
    pub(crate) fn map_value<T>(self, map: impl FnOnce(E) -> T) -> CallArgument<T> {
        CallArgument {
            value: map(self.value),
            target_param: self.target_param,
            access_mode: self.access_mode,
            passing_mode: self.passing_mode,
            span: self.span,
            target_span: self.target_span,
            marker_span: self.marker_span,
            following_separator_span: self.following_separator_span,
            parameter_slot: self.parameter_slot,
            route_future_input_type: self.route_future_input_type,
        }
    }
}

/// Arrange owned parsed arguments by their retained declaration-order slots.
///
/// WHAT: consumes each argument into its assigned slot without cloning expression handles.
/// WHY: final call validation takes ownership of the parsed list, so slot ordering can move each
///      argument directly into the validated result path.
pub(crate) fn order_owned_call_arguments_by_retained_slot<E>(
    arguments: Vec<CallArgument<E>>,
    expected_slot_count: usize,
) -> Result<Vec<Option<CallArgument<E>>>, CompilerError> {
    let mut ordered = Vec::with_capacity(expected_slot_count);
    ordered.resize_with(expected_slot_count, || None);

    for argument in arguments {
        let slot_index = retained_call_argument_slot_index(&argument, 0, expected_slot_count)?;
        let slot = &mut ordered[slot_index];
        if slot.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "Parsed call arguments retained duplicate parameter slot {}",
                slot_index
            )));
        }

        *slot = Some(argument);
    }

    Ok(ordered)
}

/// Resolve and validate one parser-assigned call-argument slot with its hidden-slot offset.
fn retained_call_argument_slot_index<E>(
    argument: &CallArgument<E>,
    slot_offset: usize,
    expected_slot_count: usize,
) -> Result<usize, CompilerError> {
    if argument.target_param.is_some() != argument.target_span.is_some() {
        return Err(CompilerError::compiler_error(
            "Parsed named call argument has incomplete target metadata",
        ));
    }

    let Some(parameter_slot) = argument.parameter_slot else {
        return Err(CompilerError::compiler_error(
            "Parsed call argument is missing its retained parameter slot",
        ));
    };
    let Some(slot_index) = parameter_slot.index().checked_add(slot_offset) else {
        return Err(CompilerError::compiler_error(
            "Parsed call argument retained an out-of-range parameter slot",
        ));
    };
    if slot_index >= expected_slot_count {
        return Err(CompilerError::compiler_error(format!(
            "Parsed call argument retained out-of-range parameter slot {}",
            slot_index
        )));
    }

    Ok(slot_index)
}

/// Borrow parsed arguments in declaration-slot order while accounting for hidden leading slots.
///
/// WHAT: validates parser-owned named-target and parameter-slot facts, then stores only references
///       in routed slots. `slot_offset` maps authored slots past hidden parameters such as a
///       receiver without cloning their expressions.
/// WHY: generic receiver and nominal inference need slot-order evidence while final validation
///      consumes the original arguments without cloning them.
pub(crate) fn order_call_argument_refs_by_retained_slot<E>(
    arguments: &[CallArgument<E>],
    expected_slot_count: usize,
    slot_offset: usize,
) -> Result<Vec<Option<&CallArgument<E>>>, CompilerError> {
    let mut ordered = Vec::with_capacity(expected_slot_count);
    ordered.resize_with(expected_slot_count, || None);

    for argument in arguments {
        let slot_index =
            retained_call_argument_slot_index(argument, slot_offset, expected_slot_count)?;
        let slot = &mut ordered[slot_index];
        if slot.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "Parsed call arguments retained duplicate parameter slot {}",
                slot_index
            )));
        }

        *slot = Some(argument);
    }

    Ok(ordered)
}

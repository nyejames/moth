//! Canonical AST call-argument metadata.
//!
//! WHAT: carries per-argument metadata for call-shaped AST nodes.
//! WHY: plain expression vectors cannot represent named-target routing or explicit call-site
//! mutable access markers, and mutable-place vs fresh-rvalue passing semantics.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
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
pub struct CallArgument {
    /// The expression value being passed as this argument.
    pub value: Expression,

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

    /// Parameter slot selected by the shared parser before this value was parsed.
    ///
    /// WHAT: retains the parser's named/positional routing decision through AST validation.
    /// WHY: final validation must not reconstruct call syntax or produce a second routing policy.
    pub(crate) parameter_slot: Option<ParameterSlot>,
}

impl CallArgument {
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
    pub fn positional(
        value: Expression,
        access_mode: CallAccessMode,
        span: Option<SourceSpan>,
    ) -> Self {
        Self {
            value,
            target_param: None,
            access_mode,
            passing_mode: Self::passing_mode_from_access_mode(access_mode),
            span,
            target_span: None,
            marker_span: None,
            parameter_slot: None,
        }
    }
    /// Build a named call argument with the default passing mode derived from `access_mode`.
    pub fn named(
        value: Expression,
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
            parameter_slot: None,
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

    /// Retain the parser-selected declaration-order parameter slot.
    pub(crate) fn with_parameter_slot(mut self, parameter_slot: ParameterSlot) -> Self {
        self.parameter_slot = Some(parameter_slot);
        self
    }
}

/// Arrange parsed arguments by their retained declaration-order slots.
///
/// WHAT: consumes parser-owned slot metadata without inspecting named targets or positional order.
/// WHY: a missing, duplicate or out-of-range slot is an internal compiler invariant failure after
pub(crate) fn order_call_arguments_by_retained_slot(
    arguments: &[CallArgument],
    expected_slot_count: usize,
) -> Result<Vec<Option<CallArgument>>, CompilerError> {
    let mut ordered = vec![None; expected_slot_count];

    for argument in arguments {
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

        let slot_index = parameter_slot.index();
        let Some(slot) = ordered.get_mut(slot_index) else {
            return Err(CompilerError::compiler_error(format!(
                "Parsed call argument retained out-of-range parameter slot {}",
                slot_index
            )));
        };

        if slot.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "Parsed call arguments retained duplicate parameter slot {}",
                slot_index
            )));
        }

        *slot = Some(argument.clone());
    }

    Ok(ordered)
}

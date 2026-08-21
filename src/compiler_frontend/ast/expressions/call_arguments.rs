//! Shared call-argument syntax and retained parameter-slot routing.
//!
//! WHAT: owns parentheses, separators, named targets, mutable markers, expression boundaries,
//!       cast-target threading and the one parser-time named/positional slot router used by every
//!       call-shaped AST surface.
//! WHY: each call consumer must parse the same syntax once and carry the selected parameter slot
//!      into final validation instead of rebuilding call meaning after expression parsing.
//!
//! This module does not own call result handling, defaults, type compatibility, access
//! validation, generic inference or call-specific AST construction. Those policies consume the
//! [`crate::compiler_frontend::ast::expressions::call_argument::ParameterSlot`] retained here.

use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    ExpectedParameterType, ParameterExpectation,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidBuiltinCallReason, InvalidCallShapeReason,
    InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// Whether a call-shaped surface accepts named arguments.
///
/// WHAT: carries the surface-specific diagnostic lane for named targets while the shared parser
///       owns all syntax and slot-selection policy.
/// WHY: source calls and constructors support named arguments, while builtin members and host
///      calls retain their existing positional-only diagnostics.
#[derive(Clone, Copy)]
pub(crate) enum NamedArgumentSyntax {
    Supported {
        callee_name: Option<StringId>,
    },
    UnsupportedCall {
        callee_name: Option<StringId>,
    },
    UnsupportedBuiltinMember {
        member_name: Option<StringId>,
        takes_no_arguments: bool,
    },
}

/// Parses a call argument list with explicit parameter expectations threaded into each argument.
///
/// WHAT: gives every argument expression a `CastTargetContext` derived from its corresponding
///      parameter type, so `cast` / `cast!` can resolve at concrete source/receiver/host
///      parameters and generic parameter slots can reject `cast` with `TargetIsGenericParameter`.
/// WHY: raw call parsing used to resolve arguments before validation. Threading expectations keeps
///      the cast-target channel narrow and local to the argument parser.
pub(crate) fn parse_call_arguments_typed_with_expectations(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    expectations: &[ParameterExpectation],
    named_arguments: NamedArgumentSyntax,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        string_table,
        CallArgumentSyntaxContext::Ordinary,
        named_arguments,
        Some(expectations),
    )
}

pub(crate) fn parse_generic_call_arguments_typed(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    generic_function_name: Option<StringId>,
    expectations: &[ParameterExpectation],
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        string_table,
        CallArgumentSyntaxContext::GenericFunction {
            function_name: generic_function_name,
        },
        NamedArgumentSyntax::Supported {
            callee_name: generic_function_name,
        },
        Some(expectations),
    )
}

#[derive(Clone, Copy)]
enum CallArgumentSyntaxContext {
    Ordinary,
    GenericFunction { function_name: Option<StringId> },
}

/// Builds a `CastTargetContext` from a single parameter expectation.
///
/// WHAT: converts the parameter's expected type into the same cast-target channel used by
///      declarations and assignments. `UnknownExternal` parameters are not builtin cast targets.
/// WHY: keeps call arguments consistent with other explicit typed boundaries without making
///      ordinary expression parsing globally type-directed.
fn cast_target_context_for_parameter_expectation(
    expectation: &ParameterExpectation,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> CastTargetContext {
    match expectation.expected_type {
        ExpectedParameterType::Known(type_id) => {
            cast_target_context_for_type_id(type_id, type_environment, string_table)
        }
        ExpectedParameterType::UnknownExternal => CastTargetContext::None,
    }
}

fn parse_call_arguments_inner(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    syntax_context: CallArgumentSyntaxContext,
    named_arguments: NamedArgumentSyntax,
    expectations: Option<&[ParameterExpectation]>,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    ast_log!("Creating function call arguments");

    if let NamedArgumentSyntax::UnsupportedBuiltinMember {
        member_name: Some(member_name),
        takes_no_arguments: true,
    } = named_arguments
    {
        if token_stream.current_token_kind() != &TokenKind::OpenParenthesis {
            return Err(CompilerDiagnostic::invalid_builtin_call(
                InvalidBuiltinCallReason::MissingParentheses,
                Some(member_name),
                token_stream.current_location(),
            )
            .into());
        }

        token_stream.advance();
        token_stream.skip_newlines();

        if token_stream.current_token_kind() != &TokenKind::CloseParenthesis {
            return Err(CompilerDiagnostic::invalid_builtin_call(
                InvalidBuiltinCallReason::TakesNoArguments,
                Some(member_name),
                token_stream.current_location(),
            )
            .into());
        }

        token_stream.advance();
        return Ok(Vec::new());
    }

    // ------------------------
    //  Consume opening paren
    // ------------------------
    if token_stream.current_token_kind() != &TokenKind::OpenParenthesis {
        return Err(CompilerDiagnostic::expected_token(
            TokenKind::OpenParenthesis,
            Some(token_stream.current_token_kind().to_owned()),
            token_stream.current_location(),
        )
        .into());
    }

    token_stream.advance();
    token_stream.skip_newlines();

    if token_stream.current_token_kind() == &TokenKind::CloseParenthesis {
        token_stream.advance();
        return Ok(Vec::new());
    }

    let mut arguments = Vec::new();
    let mut slot_router = ParameterSlotRouter::new(expectations, named_arguments);

    // ------------------------
    //  Parse each argument
    // ------------------------
    loop {
        token_stream.skip_newlines();
        if token_stream.current_token_kind() == &TokenKind::CloseParenthesis {
            token_stream.advance();
            break;
        }

        let argument_location = token_stream.current_location();

        reject_simple_generic_argument_type_ascription(token_stream, syntax_context)?;

        // Detect named-target syntax (`name = expr`) or reject unsupported variants.
        let named_target = match token_stream.current_token_kind() {
            // `~name = expr` is not supported.
            TokenKind::Mutable
                if matches!(token_stream.peek_next_token(), Some(TokenKind::Symbol(_)))
                    && token_stream
                        .tokens
                        .get(token_stream.index + 2)
                        .map(|token| &token.kind)
                        == Some(&TokenKind::Assign) =>
            {
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::Mutable,
                    token_stream.current_location(),
                )
                .into());
            }

            // Standard named argument: `name = expr`.
            TokenKind::Symbol(name)
                if token_stream.peek_next_token() == Some(&TokenKind::Assign) =>
            {
                let target_location = token_stream.current_location();
                let target_name = *name;
                token_stream.advance();
                token_stream.advance();
                token_stream.skip_newlines();
                Some((target_name, target_location))
            }

            // Parenthesized names like `(name) = expr` are not supported.
            TokenKind::OpenParenthesis
                if matches!(token_stream.peek_next_token(), Some(TokenKind::Symbol(_)))
                    && token_stream
                        .tokens
                        .get(token_stream.index + 2)
                        .map(|token| &token.kind)
                        == Some(&TokenKind::CloseParenthesis)
                    && token_stream
                        .tokens
                        .get(token_stream.index + 3)
                        .map(|token| &token.kind)
                        == Some(&TokenKind::Assign) =>
            {
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::OpenParenthesis,
                    token_stream.current_location(),
                )
                .into());
            }

            _ => None,
        };

        let parameter_slot = slot_router.route(named_target.as_ref(), argument_location.clone())?;

        let (access_mode, marker_location) =
            if token_stream.current_token_kind() == &TokenKind::Mutable {
                let marker_location = token_stream.current_location();
                token_stream.advance();
                (CallAccessMode::Mutable, Some(marker_location))
            } else {
                (CallAccessMode::Shared, None)
            };

        // A named target or access mode without a following value is an error.
        if token_stream.current_token_kind() == &TokenKind::Comma
            || token_stream.current_token_kind() == &TokenKind::CloseParenthesis
        {
            return Err(CompilerDiagnostic::unexpected_token(
                token_stream.current_token_kind().to_owned(),
                token_stream.current_location(),
            )
            .into());
        }

        let cast_target_context = parameter_slot
            .and_then(|slot| expectations.and_then(|items| items.get(slot.index())))
            .map(|expectation| {
                cast_target_context_for_parameter_expectation(
                    expectation,
                    type_interner.environment(),
                    string_table,
                )
            })
            .unwrap_or(CastTargetContext::None);
        let mut cast_target_context = cast_target_context;
        let mut inferred = ExpectedType::Infer;
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream,
                scope_context: context,
                type_interner,
                expected_type: &mut inferred,
                cast_target_context: &mut cast_target_context,
                value_mode: &ValueMode::ImmutableOwned,
                string_table,
            },
            false,
        );
        let value = create_expression_with_trailing_newline_policy(input)?;

        // `CallArgument::location` is the value-expression location. The named-parameter token
        // stays in `target_location`, and the authored `~` marker (when present) stays in
        // `marker_location`, so diagnostics can point at whichever source the author must change.
        let value_location = value.location.clone();
        let argument = if let Some((name, target_location)) = named_target {
            CallArgument::named(value, name, access_mode, value_location, target_location)
        } else {
            CallArgument::positional(value, access_mode, value_location)
        };
        let argument = if let Some(marker_location) = marker_location {
            argument.with_marker_location(marker_location)
        } else {
            argument
        };
        let argument = if let Some(parameter_slot) = parameter_slot {
            argument.with_parameter_slot(parameter_slot)
        } else {
            argument
        };
        arguments.push(argument);

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                token_stream.advance();
                token_stream.skip_newlines();
            }
            TokenKind::CloseParenthesis => {
                token_stream.advance();
                break;
            }
            _ => {
                return Err(CompilerDiagnostic::unexpected_token(
                    token_stream.current_token_kind().to_owned(),
                    token_stream.current_location(),
                )
                .into());
            }
        }
    }

    Ok(arguments)
}

/// Maintains one parser-time declaration-order routing state for a call argument list.
///
/// WHAT: selects a `ParameterSlot` before the corresponding value expression is parsed and
///       applies named-argument, duplicate and positional-order diagnostics.
/// WHY: cast targets and final validation must consume the same decision without reconstructing
///      the authored call shape later.
struct ParameterSlotRouter<'a> {
    expectations: Option<&'a [ParameterExpectation]>,
    named_arguments: NamedArgumentSyntax,
    parameter_name_to_slot: FxHashMap<StringId, usize>,
    positional_cursor: usize,
    saw_named_argument: bool,
    occupied_parameter_slots: Option<Vec<bool>>,
}

impl<'a> ParameterSlotRouter<'a> {
    fn new(
        expectations: Option<&'a [ParameterExpectation]>,
        named_arguments: NamedArgumentSyntax,
    ) -> Self {
        let parameter_name_to_slot = expectations
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .filter_map(|(index, expectation)| expectation.name.map(|name| (name, index)))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            expectations,
            named_arguments,
            parameter_name_to_slot,
            positional_cursor: 0,
            saw_named_argument: false,
            occupied_parameter_slots: expectations.map(|items| vec![false; items.len()]),
        }
    }

    fn route(
        &mut self,
        named_target: Option<&(StringId, SourceLocation)>,
        argument_location: SourceLocation,
    ) -> Result<Option<ParameterSlot>, ExpressionParseError> {
        let Some(expectations) = self.expectations else {
            if named_target.is_some() {
                self.saw_named_argument = true;
            } else {
                self.positional_cursor += 1;
            }

            return Ok(None);
        };

        if let Some((target_name, target_location)) = named_target {
            self.saw_named_argument = true;

            match self.named_arguments {
                NamedArgumentSyntax::UnsupportedCall { callee_name } => {
                    return Err(CompilerDiagnostic::invalid_call_shape(
                        InvalidCallShapeReason::NamedArgumentsNotSupported,
                        callee_name,
                        target_location.clone(),
                    )
                    .into());
                }

                NamedArgumentSyntax::UnsupportedBuiltinMember { member_name, .. } => {
                    return Err(CompilerDiagnostic::invalid_builtin_call(
                        InvalidBuiltinCallReason::NamedArgumentsNotSupported,
                        member_name,
                        target_location.clone(),
                    )
                    .into());
                }

                NamedArgumentSyntax::Supported { callee_name } => {
                    let Some(slot) = self.parameter_name_to_slot.get(target_name).copied() else {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::NamedArgumentNotFound {
                                name: *target_name,
                                known_parameters: known_parameter_names(expectations),
                            },
                            callee_name,
                            target_location.clone(),
                        )
                        .into());
                    };

                    self.mark_slot_occupied(slot, target_location.clone())?;
                    return Ok(Some(ParameterSlot::new(slot)));
                }
            }
        }

        if self.saw_named_argument {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::PositionalAfterNamed,
                self.callee_name(),
                argument_location,
            )
            .into());
        }

        let callee_name = self.callee_name();
        let Some(occupied_slots) = &mut self.occupied_parameter_slots else {
            let slot = self.positional_cursor;
            self.positional_cursor += 1;
            return Ok(Some(ParameterSlot::new(slot)));
        };

        while self.positional_cursor < occupied_slots.len()
            && occupied_slots[self.positional_cursor]
        {
            self.positional_cursor += 1;
        }

        if self.positional_cursor >= occupied_slots.len() {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::ExtraPositionalArgument {
                    expected_count: expectations.len(),
                },
                callee_name,
                argument_location,
            )
            .into());
        }

        let slot = self.positional_cursor;
        occupied_slots[slot] = true;
        self.positional_cursor += 1;
        Ok(Some(ParameterSlot::new(slot)))
    }

    fn mark_slot_occupied(
        &mut self,
        slot: usize,
        location: SourceLocation,
    ) -> Result<(), ExpressionParseError> {
        let parameter_name = self
            .expectations
            .and_then(|items| items.get(slot))
            .and_then(|expectation| expectation.name);
        let callee_name = self.callee_name();
        let Some(occupied_slots) = &mut self.occupied_parameter_slots else {
            return Ok(());
        };

        if occupied_slots[slot] {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::DuplicateArgument {
                    parameter_name,
                    parameter_index: slot,
                },
                callee_name,
                location,
            )
            .into());
        }

        occupied_slots[slot] = true;
        Ok(())
    }

    fn callee_name(&self) -> Option<StringId> {
        match self.named_arguments {
            NamedArgumentSyntax::Supported { callee_name }
            | NamedArgumentSyntax::UnsupportedCall { callee_name } => callee_name,
            NamedArgumentSyntax::UnsupportedBuiltinMember { .. } => None,
        }
    }
}

fn known_parameter_names(expectations: &[ParameterExpectation]) -> Vec<StringId> {
    expectations
        .iter()
        .filter_map(|expectation| expectation.name)
        .collect()
}

fn reject_simple_generic_argument_type_ascription(
    token_stream: &FileTokens,
    syntax_context: CallArgumentSyntaxContext,
) -> Result<(), ExpressionParseError> {
    let CallArgumentSyntaxContext::GenericFunction { function_name } = syntax_context else {
        return Ok(());
    };

    if !starts_simple_value_with_attached_type(token_stream) {
        return Ok(());
    }

    let Some(type_token) = token_stream.tokens.get(token_stream.index + 1) else {
        return Ok(());
    };

    Err(CompilerDiagnostic::invalid_generic_instantiation(
        function_name,
        InvalidGenericInstantiationReason::ExplicitCallTypeArgumentsUnsupported,
        type_token.location.clone(),
    )
    .into())
}

/// Recognize the narrow `identity(42 Int)`-style foreign syntax before the expression parser
/// tries to parse the type keyword as another expression.
///
/// This deliberately stays small: broader type-looking symbol recovery would be speculative in
/// the shared call parser and could change ordinary call errors.
fn starts_simple_value_with_attached_type(token_stream: &FileTokens) -> bool {
    let Some(value_token) = token_stream.tokens.get(token_stream.index) else {
        return false;
    };
    let Some(type_token) = token_stream.tokens.get(token_stream.index + 1) else {
        return false;
    };
    let Some(boundary_token) = token_stream.tokens.get(token_stream.index + 2) else {
        return false;
    };

    matches!(
        value_token.kind,
        TokenKind::NumericLiteral(_)
            | TokenKind::StringSliceLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::NoneLiteral
    ) && matches!(
        type_token.kind,
        TokenKind::DatatypeInt
            | TokenKind::DatatypeFloat
            | TokenKind::DatatypeBool
            | TokenKind::DatatypeString
            | TokenKind::DatatypeChar
            | TokenKind::DatatypeNone
    ) && matches!(
        boundary_token.kind,
        TokenKind::Comma | TokenKind::CloseParenthesis | TokenKind::Newline
    )
}

#[cfg(test)]
#[path = "tests/function_call_tests.rs"]
mod function_call_tests;

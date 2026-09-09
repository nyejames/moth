//! Symbol-led function-body statement parsing.
//!
//! WHAT: parses statement forms that start with a symbol inside function/start-function bodies.
//! WHY: symbol-led statements are the densest statement branch (mutation, calls, declarations,
//! access chains and start dependency callability), so isolating them keeps dispatch readable.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::function_calls::{
    ExternalFunctionCallParseInput, parse_external_function_call_expression,
};
use crate::compiler_frontend::ast::expressions::mutation::{
    handle_mutation, handle_mutation_target,
};
use crate::compiler_frontend::ast::expressions::parse_expression_places::place_expression_from_expression;
use crate::compiler_frontend::ast::field_access::parse_field_access;
use crate::compiler_frontend::ast::receiver_methods::free_function_receiver_method_call_error;
use crate::compiler_frontend::ast::statements::body_expr_stmt::{
    is_expression_statement, parse_symbol_expression_statement_candidate,
};
use crate::compiler_frontend::ast::statements::declarations::ResolvedDeclarationStatementKind;
use crate::compiler_frontend::ast::statements::declarations::new_declaration;
use crate::compiler_frontend::ast::statements::multi_bind::parse_multi_bind_statement;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::is_reserved_builtin_symbol;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidDeclarationReason,
    InvalidStandaloneStatementReason, InvalidThisUsageReason, ReservedNameOwner,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::statement_position::check_mistaken_keyword_symbol;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

// --------------------------
//  Accessed-symbol statement helper
// --------------------------

fn push_accessed_symbol_statement(
    accessed_expression: Expression,
    ast: &mut Vec<AstNode>,
    context: &ScopeContext,
    token_stream: &FileTokens,
    _symbol_id: StringId,
    _string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    if is_expression_statement(&accessed_expression) {
        ast.push(AstNode {
            span: accessed_expression.span,
            kind: NodeKind::ExpressionStatement(accessed_expression),
            scope: context.scope.clone(),
        });
        return Ok(());
    }

    // A bare field read (e.g., `obj.field`) does nothing, so it is rejected.
    if matches!(accessed_expression.kind, ExpressionKind::FieldAccess { .. }) {
        return Err(CompilerDiagnostic::invalid_standalone_statement(
            InvalidStandaloneStatementReason::FieldRead,
            Some(token_stream.current_span()),
        ));
    }

    // Any other accessed expression is also not a valid standalone statement.
    Err(CompilerDiagnostic::invalid_standalone_statement(
        InvalidStandaloneStatementReason::Expression,
        Some(token_stream.current_span()),
    ))
}

// --------------------------
//  `this` statement parsing
// --------------------------

pub(crate) fn parse_this_statement(
    token_stream: &mut FileTokens,
    ast: &mut Vec<AstNode>,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let this_id = string_table.intern("this");

    // `this` cannot be assigned when we are recovering inside a catch block.
    if context.is_assignment_target_unavailable(this_id) {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            Some(this_id),
            None,
            None,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let Some(this_reference) = context.get_reference(&this_id) else {
        return Err(CompilerDiagnostic::invalid_this_usage(
            InvalidThisUsageReason::NotInReceiverMethod,
            Some(token_stream.current_span()),
        )
        .into());
    };

    match token_stream.peek_next_token() {
        // Direct reassignment of `this` is never allowed.
        Some(next_token) if next_token.is_assignment_operator() => {
            Err(CompilerDiagnostic::invalid_this_usage(
                InvalidThisUsageReason::Reassignment,
                Some(token_stream.current_span()),
            )
            .into())
        }

        // Field access on `this`: may be a mutation (`this.x = ...`) or a
        // method/collection call (`this.x()`).
        Some(TokenKind::Dot) => {
            token_stream.advance();
            let accessed_node = parse_field_access(
                token_stream,
                this_reference.as_declaration(),
                context,
                type_interner,
                string_table,
            )?;

            if token_stream.current_token_kind().is_assignment_operator() {
                let Some(target) = place_expression_from_expression(&accessed_node) else {
                    return Err(CompilerDiagnostic::invalid_assignment_target(
                        InvalidAssignmentTargetReason::TemporaryNotAssignable,
                        None,
                        Some(accessed_node.type_id),
                        None,
                        None,
                        None,
                        Some(token_stream.current_span()),
                    )
                    .into());
                };

                let mutation_node = handle_mutation_target(
                    token_stream,
                    this_reference.as_declaration(),
                    target,
                    this_reference.binding_span(),
                    context,
                    type_interner,
                    string_table,
                )?;

                ast.push(mutation_node);
                return Ok(());
            }

            push_accessed_symbol_statement(
                accessed_node,
                ast,
                context,
                token_stream,
                this_id,
                string_table,
            )?;
            Ok(())
        }

        // Bare `this` or `this` used as the start of a general expression.
        _ => {
            let expression = parse_symbol_expression_statement_candidate(
                token_stream,
                context,
                this_id,
                type_interner,
                string_table,
            )?;

            let span = expression.span;
            ast.push(AstNode {
                kind: NodeKind::ExpressionStatement(expression),
                span,
                scope: context.scope.clone(),
            });
            Ok(())
        }
    }
}

// --------------------------
//  Symbol-led statement parsing
// --------------------------

pub(crate) fn parse_symbol_statement(
    token_stream: &mut FileTokens,
    ast: &mut Vec<AstNode>,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let TokenKind::Symbol(symbol_id) = token_stream.current_token_kind().to_owned() else {
        return Err(CompilerDiagnostic::expected_symbol_statement(Some(
            token_stream.current_span(),
        ))
        .into());
    };

    // Reject symbols that look like keywords in statement position.
    if let Some(error) = check_mistaken_keyword_symbol(symbol_id, token_stream, string_table) {
        return Err(error.into());
    }

    // Built-in type names cannot be used as value-level symbols.
    if is_reserved_builtin_symbol(string_table.resolve(symbol_id)) {
        return Err(CompilerDiagnostic::reserved_name_collision(
            symbol_id,
            ReservedNameOwner::BuiltinType,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Assignment targets are forbidden while recovering inside a catch block.
    if context.is_assignment_target_unavailable(symbol_id) {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            Some(symbol_id),
            None,
            None,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Multi-bind syntax (`a, b = ...`) takes priority over single-symbol dispatch.
    if let Some(multi_bind_node) =
        parse_multi_bind_statement(token_stream, context, type_interner, string_table)?
    {
        ast.push(multi_bind_node);
        return Ok(());
    }

    // If the symbol already names a visible local, treat it as a use
    // (assignment, field access, or expression) rather than a new declaration.
    if let Some(existing_reference) = context.get_reference(&symbol_id) {
        match token_stream.peek_next_token() {
            // Direct reassignment of an existing local variable.
            Some(next_token) if next_token.is_assignment_operator() => {
                token_stream.advance();
                let mutation_node = handle_mutation(
                    token_stream,
                    existing_reference.as_declaration(),
                    existing_reference.binding_span(),
                    context,
                    type_interner,
                    string_table,
                )?;

                ast.push(mutation_node);
                return Ok(());
            }

            // Field access on an existing local: may be a mutation or a call.
            Some(TokenKind::Dot) => {
                token_stream.advance();
                let accessed_node = parse_field_access(
                    token_stream,
                    existing_reference.as_declaration(),
                    context,
                    type_interner,
                    string_table,
                )?;

                if token_stream.current_token_kind().is_assignment_operator() {
                    let Some(target) = place_expression_from_expression(&accessed_node) else {
                        return Err(CompilerDiagnostic::invalid_assignment_target(
                            InvalidAssignmentTargetReason::TemporaryNotAssignable,
                            None,
                            Some(accessed_node.type_id),
                            None,
                            None,
                            None,
                            Some(token_stream.current_span()),
                        )
                        .into());
                    };

                    let mutation_node = handle_mutation_target(
                        token_stream,
                        existing_reference.as_declaration(),
                        target,
                        existing_reference.binding_span(),
                        context,
                        type_interner,
                        string_table,
                    )?;

                    ast.push(mutation_node);
                    return Ok(());
                }

                push_accessed_symbol_statement(
                    accessed_node,
                    ast,
                    context,
                    token_stream,
                    symbol_id,
                    string_table,
                )?;
                return Ok(());
            }

            // A type keyword after an existing symbol means the user is trying to
            // redeclare it with an explicit type, which is a shadowing error.
            Some(TokenKind::DatatypeInt)
            | Some(TokenKind::DatatypeFloat)
            | Some(TokenKind::DatatypeBool)
            | Some(TokenKind::DatatypeString)
            | Some(TokenKind::DatatypeChar)
            | Some(TokenKind::Mutable) => {
                let mut diagnostic = CompilerDiagnostic::shadowed_name(
                    symbol_id,
                    existing_reference.value.span,
                    Some(token_stream.current_span()),
                );
                diagnostic.primary_span = Some(SourceSpan::new(
                    token_stream.file_id,
                    token_stream.tokens[token_stream.index].span,
                ));
                return Err(diagnostic.into());
            }

            // Otherwise, parse the symbol as the start of a general expression statement.
            _ => {
                let expression = parse_symbol_expression_statement_candidate(
                    token_stream,
                    context,
                    symbol_id,
                    type_interner,
                    string_table,
                )?;

                let span = expression.span;
                ast.push(AstNode {
                    kind: NodeKind::ExpressionStatement(expression),
                    span,
                    scope: context.scope.clone(),
                });
                return Ok(());
            }
        }
    }

    // External (host) function calls have no local declaration; resolve by name.
    if let Some((external_function_id, external_function_def)) =
        context.lookup_visible_external_function(symbol_id)
    {
        if token_stream.peek_next_token() == Some(&TokenKind::TypeParameterBracket) {
            // Explicit external imports retain the authored dependency span; prelude-injected
            // symbols intentionally have no source span to attach.
            let previous_span = context
                .shared
                .file_visibility
                .as_ref()
                .and_then(|visibility| visibility.visible_external_symbol_spans.get(&symbol_id))
                .copied();
            return Err(CompilerDiagnostic::duplicate_declaration(
                symbol_id,
                previous_span,
                Some(token_stream.current_span()),
            )
            .into());
        }

        let call_span = Some(token_stream.current_span());
        let external_call_expression =
            parse_external_function_call_expression(ExternalFunctionCallParseInput {
                token_stream,
                external_function_id,
                external_function: external_function_def,
                call_span,
                context,
                value_required: false,
                allow_boundary_catch: true,
                warnings: Some(warnings),
                type_interner,
                string_table,
            })?;
        let external_call_span = external_call_expression.span;
        ast.push(AstNode {
            kind: NodeKind::ExpressionStatement(external_call_expression),
            span: external_call_span,
            scope: context.scope.clone(),
        });
        return Ok(());
    }

    // An open parenthesis after an unknown symbol means a call attempt.
    // Provide targeted diagnostics for receiver methods and external types.
    if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis) {
        if let Some(receiver_method_entry) =
            context.lookup_visible_receiver_method_by_name(symbol_id)
        {
            return Err(free_function_receiver_method_call_error(
                symbol_id,
                receiver_method_entry,
                Some(token_stream.current_span()),
                string_table,
            )
            .into());
        }

        if context.lookup_visible_external_type(symbol_id).is_some() {
            return Err(CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::ExternalTypeLiteralConstruction,
                Some(symbol_id),
                Some(token_stream.current_span()),
            )
            .into());
        }

        return Err(CompilerDiagnostic::unknown_value_name(
            symbol_id,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Namespace-record calls such as `canvas.fill_rect(...)` have no local binding for the
    // namespace symbol, but they are valid side-effect statements when the field access resolves
    // to a call. Route them through expression-statement validation before declaration parsing
    // interprets the leading symbol as a malformed declaration.
    if token_stream.peek_next_token() == Some(&TokenKind::Dot) {
        let expression = parse_symbol_expression_statement_candidate(
            token_stream,
            context,
            symbol_id,
            type_interner,
            string_table,
        )?;
        let span = expression.span;
        ast.push(AstNode {
            kind: NodeKind::ExpressionStatement(expression),
            span,
            scope: context.scope.clone(),
        });
        return Ok(());
    }

    // No existing reference and no call target: this must be a new declaration.
    let resolved_declaration = new_declaration(
        token_stream,
        symbol_id,
        context,
        type_interner,
        warnings,
        string_table,
    )?;
    let declaration = resolved_declaration.declaration;
    let statement_kind = resolved_declaration.statement_kind;
    let is_compile_time_binding = resolved_declaration.is_compile_time_binding;
    let declaration_span = resolved_declaration.binding_span;

    // Lift struct definitions and functions to the AST statement level;
    // everything else becomes a local variable declaration.
    match &statement_kind {
        ResolvedDeclarationStatementKind::StructDefinition(params) => {
            ast.push(AstNode {
                kind: NodeKind::StructDefinition(declaration.id.to_owned(), params.to_owned()),
                span: declaration_span,
                scope: context.scope.clone(),
            });
        }

        ResolvedDeclarationStatementKind::Function { signature, body } => {
            ast.push(AstNode {
                kind: NodeKind::Function(
                    declaration.id.to_owned(),
                    signature.to_owned(),
                    body.to_owned(),
                ),
                span: declaration_span,
                scope: context.scope.clone(),
            });
        }

        ResolvedDeclarationStatementKind::Variable => {
            ast.push(AstNode {
                kind: NodeKind::VariableDeclaration(declaration.to_owned()),
                span: declaration_span,
                scope: context.scope.clone(),
            });
        }
    }

    if is_compile_time_binding {
        context.add_compile_time_var(declaration, declaration_span);
    } else {
        context.add_var(declaration, declaration_span);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/struct_parsing_tests.rs"]
mod struct_parsing_tests;

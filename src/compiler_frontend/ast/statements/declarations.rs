//! Body-local declaration parsing and lowering.
//!
//! WHAT: parses declarations that appear inside executable AST bodies, resolves their declared
//! type boundary, parses the initializer, and emits the resulting local `Declaration`.
//! WHY: top-level declaration discovery belongs to headers/environment construction; this module
//! owns only source-order body declarations and the coercion boundary between an initializer
//! expression and the declared local type.

use crate::ast_log;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::statements::collections::new_collection;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, SignatureTypeFallbackPolicy, signature_member_to_declaration,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionContextInputs, fold_collection_capacity,
    resolve_diagnostic_type_to_type_id_checked, resolve_parsed_type_annotation,
};
use crate::compiler_frontend::ast::{
    ContextKind, ScopeContext,
    ast_nodes::Declaration,
    expressions::parse_expression::create_expression_with_trailing_newline_policy,
    statements::value_production::{ValueReceiverKind, try_parse_value_block_at_receiver},
};
use crate::compiler_frontend::builtins::error_type::is_reserved_builtin_symbol;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, InvalidCollectionTypeReason,
    InvalidDeclarationReason, InvalidExpressionReason, InvalidFallibleHandlingReason,
    TypeMismatchContext,
};

use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::datatypes::{DataType, ReceiverKey};
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, parse_declaration_syntax,
};
use crate::compiler_frontend::declaration_syntax::r#struct::{
    parse_struct_shell, validate_struct_default_values,
};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::signature_position::check_signature_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{
    FilePathSyntax, FileTokens, SourceLocation, Token, TokenKind,
};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedCollectionContext, ExpectedType, cast_target_context_for_type_id,
    parse_expectation_for_type_id,
};
use crate::compiler_frontend::utilities::token_scan::pipe_opens_anonymous_record;

/// Body-local declaration parsing shares the AST body error lane.
///
/// A declaration initializer or signature default can require a temporary substream over a
/// frozen file table. If that retained-table invariant fails, the error must reach module
/// emission as `CompilerError`, not become an authored declaration diagnostic.
type DeclarationResult<T> = Result<T, ExpressionParseError>;

/// Returns `Some(capacity)` when the parsed type is a capacity-only shorthand `{N}`.
///
/// WHAT: detects shorthand collection annotations where the element type is inferred.
/// WHY: shorthand declarations need special handling to fold capacity first and
///      parse the initializer as a collection literal with capacity context.
fn capacity_only_shorthand(type_ref: &ParsedTypeRef) -> Option<&ParsedCollectionCapacity> {
    match type_ref {
        ParsedTypeRef::Collection {
            element,
            fixed_capacity: Some(capacity),
            ..
        } if matches!(element.as_ref(), ParsedTypeRef::Inferred) => Some(capacity),
        _ => None,
    }
}

/// Classify a body-local constant initializer through the module's effective TIR views.
///
/// Both declaration paths use the shared module store so templates retain their exact view
/// identity after composition. Classification does not rebuild a scratch template view.
fn initializer_is_compile_time_constant(
    initializer: &Expression,
    context: &ScopeContext,
) -> Result<bool, TemplateError> {
    initializer
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, &context.template_ir_store)
        })
        .map(|kind| kind.is_compile_time_value())
}

/// Apply binding-level reactive identity after the initializer has been fully typed.
///
/// WHAT: `$Type`/`$=` declarations become stable reactive sources; ordinary declarations store a
/// snapshot even when their initializer read a reactive source.
/// WHY: reactive identity is declaration metadata, not part of `TypeId` or the initializer's
/// natural expression type.
fn apply_reactive_declaration_metadata(
    value: &mut Expression,
    is_reactive_binding: bool,
    qualified_name: &InternedPath,
) {
    if is_reactive_binding {
        value.reactive_source = Some(ReactiveSource {
            path: qualified_name.clone(),
            kind: ReactiveSourceKind::Declaration,
        });
    } else {
        value.clear_reactive_source();
    }
}

/// Body-local declaration plus syntax-origin facts that are not stored on `Declaration`.
///
/// WHAT: carries whether the user authored the binding with `#`.
/// WHY: fixed-capacity type syntax accepts bare explicit constants only, so body
///      parsing must preserve the distinction between `#` constants and foldable
///      runtime immutable bindings while registering locals.
pub(crate) struct ResolvedDeclaration {
    pub(crate) declaration: Declaration,
    pub(crate) statement_kind: ResolvedDeclarationStatementKind,
    pub(crate) is_compile_time_binding: bool,
    pub(crate) binding_location: SourceLocation,
}

/// Statement-level shape that declaration parsing discovered alongside the binding value.
///
/// WHAT: keeps function bodies owned by statement emission while the declaration value remains a
/// signature-only callable expression for lookup and type checking.
pub(crate) enum ResolvedDeclarationStatementKind {
    Variable,
    StructDefinition(Vec<Declaration>),
    Function {
        signature: FunctionSignature,
        body: Vec<AstNode>,
    },
}

/// Parse a new body-local declaration from the token stream.
///
/// Handles function declarations as a fast path (they use a dedicated signature/body syntax)
/// before falling through to generic value declaration parsing via `resolve_declaration_syntax`.
pub(crate) fn new_declaration(
    token_stream: &mut FileTokens,
    symbol_id: StringId,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> DeclarationResult<ResolvedDeclaration> {
    let declaration_name = string_table.resolve(symbol_id).to_owned();
    ensure_not_keyword_shadow_identifier(symbol_id, token_stream.current_location(), string_table)?;

    if is_reserved_builtin_symbol(&declaration_name) {
        return Err(CompilerDiagnostic::invalid_declaration(
            InvalidDeclarationReason::ReservedBuiltinName,
            Some(symbol_id),
            token_stream.current_location(),
        )
        .into());
    }

    // Capture the authored binding-name location before advancing past the name token.
    // This differs from `declaration.value.location` (the initializer expression location)
    // and is used for immutable-assignment secondary labels pointing at the original binding.
    let binding_location = token_stream.current_location();

    // Move past the name
    token_stream.advance();

    let qualified_name = context.scope.to_owned().append(symbol_id);

    // ----------------------------
    //  Function declaration fast-path
    // ----------------------------
    // Function declarations are parsed eagerly here because they use
    // a dedicated signature/body syntax that does not fit value declarations.
    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        if let Some(warning) = naming_warning_for_identifier(
            symbol_id,
            token_stream.current_location(),
            IdentifierNamingKind::ValueLike,
            string_table,
        ) {
            context.emit_warning(warning);
        }

        let function_signature = FunctionSignature::new(
            token_stream,
            warnings,
            string_table,
            &qualified_name,
            context,
            type_interner,
        )?;
        let function_context =
            context.new_child_function(symbol_id, function_signature.to_owned(), string_table);

        let function_body = function_body_to_ast(
            token_stream,
            function_context,
            type_interner,
            warnings,
            string_table,
        )?;
        let receiver = function_signature_receiver(&function_signature, string_table);
        let function_data_type =
            DataType::Function(Box::new(receiver.clone()), function_signature.clone());
        let function_type_id = resolve_diagnostic_type_to_type_id_checked(
            &function_data_type,
            type_interner.environment_mut_for_derived_types(),
            &token_stream.current_location(),
        )?;

        return Ok(ResolvedDeclaration {
            declaration: Declaration {
                id: qualified_name,
                value: Expression::function(
                    receiver,
                    function_signature.to_owned(),
                    function_type_id,
                    token_stream.current_location(),
                ),
            },
            statement_kind: ResolvedDeclarationStatementKind::Function {
                signature: function_signature,
                body: function_body,
            },
            is_compile_time_binding: false,
            binding_location,
        });
    }

    if let Some(error) = check_signature_common_mistake(token_stream) {
        return Err(error.into());
    }

    // ----------------------------
    //  Parse declaration syntax
    // ----------------------------
    let declaration_syntax = parse_declaration_syntax(token_stream, symbol_id, string_table)?;

    // Heuristic: a leading type-parameter pipe after the binding marker indicates
    // a struct or generic type definition, which uses type-like naming conventions.
    let naming_kind = if matches!(
        declaration_syntax
            .initializer_tokens
            .first()
            .map(|token| &token.kind),
        Some(TokenKind::TypeParameterBracket)
    ) {
        IdentifierNamingKind::TypeLike
    } else {
        IdentifierNamingKind::ValueLike
    };

    if let Some(warning) = naming_warning_for_identifier(
        symbol_id,
        declaration_syntax.location.to_owned(),
        naming_kind,
        string_table,
    ) {
        context.emit_warning(warning);
    }

    let is_compile_time_binding = declaration_syntax.binding_mode.is_compile_time();
    let declaration = resolve_declaration_syntax(
        declaration_syntax,
        qualified_name,
        &token_stream.path_syntax,
        &mut *context,
        type_interner,
        string_table,
    )?;
    let statement_kind = match &declaration.value.kind {
        ExpressionKind::StructDefinition(params) => {
            ResolvedDeclarationStatementKind::StructDefinition(params.to_owned())
        }
        _ => ResolvedDeclarationStatementKind::Variable,
    };

    Ok(ResolvedDeclaration {
        declaration,
        statement_kind,
        is_compile_time_binding,
        binding_location,
    })
}

/// Extract the receiver key from a function signature when the first parameter is named `this`.
fn function_signature_receiver(
    signature: &FunctionSignature,
    string_table: &mut StringTable,
) -> Option<ReceiverKey> {
    let this_name = string_table.intern("this");
    signature
        .parameters
        .first()
        .filter(|parameter| parameter.id.name() == Some(this_name))
        .and_then(|parameter| parameter.value.diagnostic_type.receiver_key_from_type())
}

/// Resolve a parsed declaration syntax into a fully typed `Declaration`.
///
/// This is the main lowering path for body-local value and struct declarations.
/// It resolves the declared type annotation, parses the initializer expression,
/// validates type compatibility, and applies contextual coercion.
/// Names the runtime storage position a declaration creates, or `None` when it is a constant.
///
pub fn resolve_declaration_syntax(
    declaration_syntax: DeclarationSyntax,
    qualified_name: InternedPath,
    path_syntax: &FilePathSyntax,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> DeclarationResult<Declaration> {
    // ----------------------------
    //  Validate constant-context constraints
    // ----------------------------
    let is_reactive_binding = declaration_syntax.binding_mode.is_reactive();
    let value_mode = declaration_syntax.value_mode();
    if declaration_syntax.binding_mode.is_mutable() && context.kind.is_constant_context() {
        return Err(CompilerDiagnostic::invalid_declaration(
            InvalidDeclarationReason::ConstantCannotBeMutable,
            None,
            declaration_syntax.location.clone(),
        )
        .into());
    }

    // ----------------------------
    //  Resolve declared type
    // ----------------------------
    let declaration_location = declaration_syntax.location.clone();

    // Capacity-only shorthand (`{N}`) requires special handling: the element type must be
    // inferred from the initializer literal, so we intercept before normal resolution.
    if let Some(capacity) = capacity_only_shorthand(&declaration_syntax.type_annotation) {
        let folded_capacity = fold_collection_capacity(
            capacity,
            Some(context),
            type_interner.environment_mut_for_derived_types(),
        )
        .map_err(|diagnostic| diagnostic.into_boxed())?;

        let mut initializer_tokens = declaration_syntax.initializer_tokens.clone();
        initializer_tokens.push(Token::new(
            TokenKind::Eof,
            declaration_syntax.location.to_owned(),
        ));
        let mut initializer_stream = FileTokens::new_from_slice(
            qualified_name.to_owned(),
            None,
            None,
            initializer_tokens,
            path_syntax,
        )
        .map_err(ExpressionParseError::from)?;

        // Shorthand requires an immediate collection literal initializer.
        if initializer_stream.current_token_kind() != &TokenKind::OpenCurly {
            return Err(CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
                initializer_stream.current_location(),
            )
            .into());
        }

        let collection_context = ExpectedCollectionContext::CapacityOnlyShorthand {
            fixed_capacity: folded_capacity,
        };
        let mut parsed_initializer = new_collection(
            &mut initializer_stream,
            collection_context,
            context,
            type_interner,
            &value_mode,
            string_table,
        )?;

        // The collection literal parser stops at the closing `}` without consuming it.
        // Advance past it so the remainder of the declaration validation sees EOF.
        if initializer_stream.current_token_kind() == &TokenKind::CloseCurly {
            initializer_stream.advance();
        }

        // Shorthand already rejected empty literals during parsing, but immutable
        // empty fixed collections are also invalid for explicit fixed annotations.
        // Post-parse validation for token consumption and constant folding.
        let initializer_is_compile_time_constant =
            if declaration_syntax.binding_mode.is_compile_time() {
                initializer_is_compile_time_constant(&parsed_initializer, context)?
            } else {
                true
            };

        if declaration_syntax.binding_mode.is_compile_time()
            && !initializer_is_compile_time_constant
        {
            return Err(CompilerDiagnostic::compile_time_evaluation_error(
                CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
                qualified_name.name(),
                declaration_syntax.location.clone(),
            )
            .into());
        }

        initializer_stream.skip_newlines();
        if initializer_stream.current_token_kind() != &TokenKind::Eof {
            return Err(CompilerDiagnostic::unexpected_token(
                initializer_stream.current_token_kind().to_owned(),
                initializer_stream.current_location(),
            )
            .into());
        }

        parsed_initializer.value_mode = value_mode.to_owned();
        apply_reactive_declaration_metadata(
            &mut parsed_initializer,
            is_reactive_binding,
            &qualified_name,
        );

        return Ok(Declaration {
            id: qualified_name,
            value: parsed_initializer,
        });
    }

    let resolved_annotation = {
        let mut type_resolution_context =
            TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
                declaration_table: &context.top_level_declarations,
                visible_declaration_ids: context.visible_declaration_ids.as_ref(),
                visible_external_symbols: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_external_symbols),
                visible_source_bindings: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_source_names),
                visible_type_aliases: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_type_alias_names),
                resolved_type_aliases: context.resolved_type_aliases.as_deref(),
                generic_declarations_by_path: context.generic_declarations_by_path.as_deref(),
                resolved_struct_fields_by_path: context.resolved_struct_fields_by_path.as_deref(),
                type_environment: type_interner.environment_mut_for_derived_types(),
                visible_namespace_records: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_namespace_records),
                trait_environment: Some(context.trait_environment()),
                trait_evidence_environment: Some(context.trait_evidence_environment()),
                visible_trait_names: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_trait_names),
            })
            .with_active_generic_type_context(context.active_generic_type_context());
        resolve_parsed_type_annotation(
            declaration_syntax.semantic_type(),
            &declaration_location,
            &mut type_resolution_context,
            string_table,
            Some(context),
        )?
    };

    let mut initializer_tokens = declaration_syntax.initializer_tokens;
    initializer_tokens.push(Token::new(
        TokenKind::Eof,
        declaration_syntax.location.to_owned(),
    ));
    let mut initializer_stream = FileTokens::new_from_slice(
        qualified_name.to_owned(),
        None,
        None,
        initializer_tokens,
        path_syntax,
    )
    .map_err(ExpressionParseError::from)?;

    // Check the first token before dispatching so we don't wastefully call
    // `create_expression` recursively when the initializer is a struct definition.

    let mut parsed_initializer = match initializer_stream.current_token_kind() {
        // Struct Definition
        //
        // Anonymous const records own `| name = expr |` initializers in both binding modes;
        // everything else after `|` stays with the struct shell grammar.
        TokenKind::TypeParameterBracket
            if !pipe_opens_anonymous_record(
                &initializer_stream.tokens,
                initializer_stream.index,
            ) =>
        {
            // Struct field defaults must be compile-time foldable, so they are parsed
            // in a dedicated constant context.
            let constant_context =
                ScopeContext::new_constant(initializer_stream.src_path.to_owned(), context);
            let owner_path = initializer_stream.src_path.to_owned();
            let mut field_warnings = Vec::new();
            let field_syntax = parse_struct_shell(
                &mut initializer_stream,
                string_table,
                &mut field_warnings,
                &owner_path,
            )?;
            for warning in field_warnings {
                context.emit_warning(warning);
            }

            let mut params = Vec::with_capacity(field_syntax.len());
            for field in &field_syntax {
                params.push(signature_member_to_declaration(
                    field,
                    path_syntax,
                    &constant_context,
                    type_interner,
                    string_table,
                    SignatureTypeFallbackPolicy::StrictCapacity,
                )?);
            }

            validate_struct_default_values(&params, &context.template_ir_store)
                .map_err(ExpressionParseError::from)?;

            Expression::struct_definition(
                params,
                initializer_stream.current_location(),
                value_mode.to_owned(),
            )
        }

        _ => {
            // Keep the canonical annotation TypeId beside the expression so
            // compatibility and coercion do not re-derive semantic identity from
            // diagnostic `DataType` spelling.
            //
            // Pass parse-time context only where syntax requires it, such as
            // `none` and empty collection literals. Other expressions resolve
            // their natural type before this declaration boundary validates and
            // coerces them.
            let declared_type_id = resolved_annotation.type_id;
            let mut expression_type = declared_type_id
                .map(|type_id| parse_expectation_for_type_id(type_id, type_interner.environment()))
                .unwrap_or(ExpectedType::Infer);
            let mut cast_target_context = declared_type_id
                .map(|type_id| {
                    cast_target_context_for_type_id(
                        type_id,
                        type_interner.environment(),
                        string_table,
                    )
                })
                .unwrap_or(CastTargetContext::None);

            // `DataType::Inferred` is a parse-level marker for omitted type annotations.
            // When the type is inferred, the initializer expression inherits the parent
            // context's expected result types; otherwise it is constrained to the
            // resolved declared type.
            let expression_expected_results = if let Some(declared_type_id) = declared_type_id {
                vec![declared_type_id]
            } else {
                context.expected_result_type_ids.clone()
            };
            let mut expression_context = context.new_child_expression(expression_expected_results);

            // Body-local compile-time constants need the same constant-reference rules
            // as top-level constants, but top-level header constants must keep their
            // stronger `ConstantHeader` context for const-record coercion.
            if declaration_syntax.binding_mode.is_compile_time()
                && !context.kind.is_constant_context()
            {
                expression_context.kind = ContextKind::Constant;
            } else {
                expression_context.kind = context.kind.clone();
            }

            let expression = if let Some(value_block_result) = try_parse_value_block_at_receiver(
                &mut initializer_stream,
                &expression_context,
                type_interner,
                &expression_context.expected_result_type_ids,
                ValueReceiverKind::Declaration,
                string_table,
            ) {
                value_block_result?
            } else {
                let input = ExpressionParseInput::ordinary(
                    ExpressionParseResources {
                        token_stream: &mut initializer_stream,
                        scope_context: &expression_context,
                        type_interner,
                        expected_type: &mut expression_type,
                        cast_target_context: &mut cast_target_context,
                        value_mode: &value_mode,
                        string_table,
                    },
                    false,
                );
                create_expression_with_trailing_newline_policy(input)?
            };

            if let Some(declared_type_id) = declared_type_id {
                // This is an explicit typed boundary: apply ordinary contextual coercions in
                // one shared path.
                coerce_expression_to_explicit_type_boundary(
                    expression,
                    declared_type_id,
                    type_interner.environment(),
                    context,
                    TypeMismatchContext::Declaration,
                )?
            } else {
                expression
            }
        }
    };

    // Body-local compile-time constants must fully fold after parsing and coercion.
    // Top-level constants are validated by `ConstantResolutionSession`;
    // this check covers the body-local path through `resolve_declaration_syntax`.
    let initializer_is_compile_time_constant = if declaration_syntax.binding_mode.is_compile_time()
    {
        initializer_is_compile_time_constant(&parsed_initializer, context)?
    } else {
        true
    };

    if declaration_syntax.binding_mode.is_compile_time() && !initializer_is_compile_time_constant {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
            qualified_name.name(),
            declaration_syntax.location.clone(),
        )
        .into());
    }

    // Defensive: ensure the initializer parser consumed all tokens.
    // If tokens remain, the parser stopped early (e.g. a newline broke the
    // expression before it was complete). This prevents silent truncation.
    initializer_stream.skip_newlines();
    if initializer_stream.current_token_kind() == &TokenKind::Else
        && type_interner
            .environment()
            .option_inner_type(parsed_initializer.type_id)
            .is_some()
    {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::DirectOptionFallbackSyntax,
            initializer_stream.current_location(),
        )
        .into());
    }

    if initializer_stream.current_token_kind() != &TokenKind::Eof {
        if matches!(
            initializer_stream.current_token_kind(),
            TokenKind::TypeParameterBracket
        ) && matches!(
            parsed_initializer.kind,
            ExpressionKind::AnonymousConstRecord { .. }
        ) {
            // Extra `|` after a complete record is the usual leftover from an
            // inline nested `|...|` that the parser already closed.
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::NestedAnonymousConstRecord,
                initializer_stream.current_location(),
            )
            .into());
        }

        return Err(CompilerDiagnostic::unexpected_token(
            initializer_stream.current_token_kind().to_owned(),
            initializer_stream.current_location(),
        )
        .into());
    }

    // Reject immutable bindings initialized with an empty fixed collection literal.
    // Mutable bindings are allowed because the collection may be filled later.
    if !value_mode.is_mutable()
        && let Some(type_id) = resolved_annotation.type_id
    {
        let env = type_interner.environment();
        if env.collection_fixed_capacity(type_id).is_some()
            && let ExpressionKind::Collection(items) = &parsed_initializer.kind
            && items.is_empty()
        {
            return Err(CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::EmptyImmutableFixedCollection,
                parsed_initializer.location.clone(),
            )
            .into());
        }
    }

    // WHAT: the binding marker (`~=` / `=`) is the single source of truth for whether the
    // stored declaration is a mutable place.
    // WHY: rvalue initializers (for example struct literals and collections) inherit ownership
    // from type defaults; preserving that ownership would incorrectly allow writes through
    // immutable bindings and mutable receiver calls.
    parsed_initializer.value_mode = value_mode.to_owned();
    apply_reactive_declaration_metadata(
        &mut parsed_initializer,
        is_reactive_binding,
        &qualified_name,
    );

    ast_log!(
        "Created new ",
        Cyan #value_mode,
        " ",
        resolved_annotation.diagnostic_type.display_with_table(string_table)
    );

    Ok(Declaration {
        id: qualified_name,
        value: parsed_initializer,
    })
}

#[cfg(test)]
#[path = "tests/declaration_tests.rs"]
mod declaration_tests;

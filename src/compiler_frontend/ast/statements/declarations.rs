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
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::module_ast::environment::config_resolution::expression_for_resolved_build_config_value;
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
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticToken,
    InvalidCollectionTypeReason, InvalidConfigReason, InvalidDeclarationReason,
    InvalidExpressionReason, InvalidFallibleHandlingReason, TypeMismatchContext,
};

use crate::compiler_frontend::build_config::BuildInputName;
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::datatypes::{DataType, ReceiverKey};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, parse_declaration_syntax,
};
use crate::compiler_frontend::declaration_syntax::r#struct::{
    parse_struct_shell, validate_struct_default_values,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::signature_position::check_signature_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{TokenRange, TokenTag};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedCollectionContext, ExpectedType, cast_target_context_for_type_id,
    parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Body-local declaration parsing shares the AST body error lane.
///
/// A declaration initializer or signature default can require a temporary substream over a
/// frozen file table. If that retained-table invariant fails, the error must reach module
/// emission as `CompilerError`, not become an authored declaration diagnostic.
type DeclarationResult<T> = Result<T, ExpressionParseError>;

/// True when `|` at `pipe_index` opens a value record in the bounded declaration cursor view.
///
/// WHAT: classifies the statement-owned struct/record dispatch from canonical `TokenTag` facts.
/// WHY: initializer substreams remain bounded views, so cursor-relative indexes and source spans
/// stay tied to the same source-token owner.
fn pipe_opens_value_record_at_cursor(
    cursor: &DeclarationCursor,
    pipe_index: usize,
    compile_time: bool,
) -> bool {
    if compile_time {
        return true;
    }
    let tag_at = |probe: usize| cursor.token_tag_at(probe);
    let skip_newlines = |mut probe: usize| {
        while matches!(tag_at(probe), Some(TokenTag::NEWLINE)) {
            probe += 1;
        }
        probe
    };
    let mut probe = skip_newlines(pipe_index + 1);
    if matches!(tag_at(probe), Some(TokenTag::TYPE_PARAMETER_BRACKET)) {
        return false;
    }
    if !matches!(tag_at(probe), Some(TokenTag::SYMBOL)) {
        return false;
    }
    probe = skip_newlines(probe + 1);
    matches!(
        tag_at(probe),
        Some(TokenTag::ASSIGN) | Some(TokenTag::COMMA)
    )
}

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
/// Inspect the first authored initializer token without retaining a token vector on the shell.
fn initializer_starts_with_type_parameter(source: &AstCursor, range: Option<TokenRange>) -> bool {
    let Some(range) = range else {
        return false;
    };
    matches!(
        source
            .token_ref_at(range.start().index())
            .map(|token| token.tag()),
        Some(TokenTag::TYPE_PARAMETER_BRACKET)
    )
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
/// Classify config-qualified fields as deferred placeholders while preserving ordinary compile-time
/// checking for every other field. The compiler config resolver validates and replaces these fields
/// before the top-level constant is folded.
fn initializer_is_compile_time_constant_with_config_placeholders(
    initializer: &Expression,
    context: &ScopeContext,
) -> Result<bool, TemplateError> {
    let ExpressionKind::AnonymousConstRecord { fields } = &initializer.kind else {
        return initializer_is_compile_time_constant(initializer, context);
    };

    for field in fields {
        if field.config_qualifier.is_some() {
            continue;
        }
        if !initializer_is_compile_time_constant(&field.value, context)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Reject config-qualified fields at the owning body-local record declaration.
///
/// This intentionally inspects only direct record fields. Nested expression walks are not a
/// placement boundary: each declaration/record parser owns the metadata it parsed.
fn reject_config_qualifiers_on_record_fields(
    initializer: &Expression,
    path_fork: &PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let ExpressionKind::AnonymousConstRecord { fields } = &initializer.kind else {
        return Ok(());
    };

    for field in fields {
        if let Some(qualifier) = &field.config_qualifier {
            let mut diagnostic = CompilerDiagnostic::invalid_config_reason(
                path_fork.component(field.id),
                InvalidConfigReason::ConfigQualifierInvalidPlacement,
                qualifier.qualifier_span,
            );
            diagnostic.primary_span = qualifier.qualifier_span;
            return Err(diagnostic.into());
        }
    }

    Ok(())
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
    qualified_name: &PathId,
) {
    if is_reactive_binding {
        value.reactive_source = Some(ReactiveSource {
            path: *qualified_name,
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
    pub(crate) binding_span: Option<SourceSpan>,
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

/// Mutable interner tables shared by declaration lowering.
///
/// WHAT: groups the string and path tables that every declaration-lowering call needs together.
/// WHY: keeping them together keeps `resolve_declaration_syntax` under the argument-count lint
/// without changing lowering behavior.
pub(crate) struct DeclarationLoweringTables<'a> {
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Parse a new body-local declaration from the token stream.
///
/// Handles function declarations as a fast path (they use a dedicated signature/body syntax)
/// before falling through to generic value declaration parsing via `resolve_declaration_syntax`.
pub(crate) fn new_declaration(
    token_stream: &mut AstCursor,
    symbol_id: StringId,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DeclarationResult<ResolvedDeclaration> {
    let declaration_name = string_table.resolve(symbol_id).to_owned();
    ensure_not_keyword_shadow_identifier(
        symbol_id,
        Some(token_stream.current_span()),
        string_table,
    )?;

    if is_reserved_builtin_symbol(&declaration_name) {
        return Err(CompilerDiagnostic::invalid_declaration(
            InvalidDeclarationReason::ReservedBuiltinName,
            Some(symbol_id),
            Some(token_stream.current_span()),
        )
        .into());
    }
    // Capture the authored binding-name span before advancing past the name token.
    let binding_span = Some(token_stream.current_span());

    // Move past the name
    token_stream.advance();

    let qualified_name = path_fork
        .try_intern_child(context.scope, symbol_id)
        .expect("path table exhausted while creating declaration scope");

    // ----------------------------
    //  Function declaration fast-path
    // ----------------------------
    // Function declarations are parsed eagerly here because they use
    // a dedicated signature/body syntax that does not fit value declarations.
    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        if let Some(warning) = naming_warning_for_identifier(
            symbol_id,
            Some(token_stream.current_span()),
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
            path_fork,
        )?;
        let function_context = context.new_child_function(
            symbol_id,
            function_signature.to_owned(),
            string_table,
            path_fork,
        );

        let function_body = function_body_to_ast(
            token_stream,
            function_context,
            type_interner,
            warnings,
            string_table,
            path_fork,
        )?;
        let receiver = function_signature_receiver(&function_signature, string_table, path_fork);
        let function_data_type =
            DataType::Function(Box::new(receiver.clone()), function_signature.clone());
        let function_type_id = resolve_diagnostic_type_to_type_id_checked(
            &function_data_type,
            type_interner.environment_mut_for_derived_types(),
            Some(token_stream.current_span()),
        )?;

        return Ok(ResolvedDeclaration {
            declaration: Declaration {
                id: qualified_name,
                value: Expression::function(
                    receiver,
                    function_signature.to_owned(),
                    function_type_id,
                    binding_span,
                ),
                binding_span,
                config_qualifier: None,
            },
            statement_kind: ResolvedDeclarationStatementKind::Function {
                signature: function_signature,
                body: function_body,
            },
            is_compile_time_binding: false,
            binding_span,
        });
    }

    if let Some(error) = check_signature_common_mistake(token_stream) {
        return Err(error.into());
    }

    // ----------------------------
    //  Parse declaration syntax
    // ----------------------------
    let mut span_builder = ExtendedSpanBuilder::new();
    let declaration_syntax = {
        let (syntax, next_index) = {
            let mut declaration_cursor = token_stream.declaration_cursor()?;
            let syntax = parse_declaration_syntax(
                &mut declaration_cursor,
                symbol_id,
                string_table,
                &mut span_builder,
            )?;
            (syntax, declaration_cursor.position())
        };
        token_stream.set_position(next_index)?;
        syntax
    };

    // Heuristic: a leading type-parameter pipe after the binding marker indicates
    // a struct or generic type definition, which uses type-like naming conventions.
    let naming_kind = if initializer_starts_with_type_parameter(
        token_stream,
        declaration_syntax.initializer_range,
    ) {
        IdentifierNamingKind::TypeLike
    } else {
        IdentifierNamingKind::ValueLike
    };

    if let Some(warning) = naming_warning_for_identifier(
        symbol_id,
        declaration_syntax.span,
        naming_kind,
        string_table,
    ) {
        context.emit_warning(warning);
    }
    let is_compile_time_binding = declaration_syntax.binding_mode.is_compile_time();
    let mut declaration = resolve_declaration_syntax(
        declaration_syntax,
        qualified_name,
        Some(token_stream),
        &mut *context,
        type_interner,
        DeclarationLoweringTables {
            string_table,
            path_fork,
        },
    )?;
    // The binding anchor is the declaration-name token captured on entry, not the
    // initializer value span. `resolve_declaration_syntax` leaves it empty for this
    // body-local path; restore the exact anchor here.
    declaration.binding_span = binding_span;
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
        binding_span,
    })
}
/// Extract the receiver key from a function signature when the first parameter is named `this`.
fn function_signature_receiver(
    signature: &FunctionSignature,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Option<ReceiverKey> {
    let this_name = string_table.intern("this");
    signature
        .parameters
        .first()
        .filter(|parameter| path_fork.component(parameter.id) == Some(this_name))
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
    qualified_name: PathId,
    source_owner: Option<&AstCursor>,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    tables: DeclarationLoweringTables<'_>,
) -> DeclarationResult<Declaration> {
    let DeclarationLoweringTables {
        string_table,
        path_fork,
    } = tables;
    let mut span_builder = ExtendedSpanBuilder::new();
    let config_qualifier = declaration_syntax.config_qualifier.clone();
    let config_constant_context = matches!(context.kind, ContextKind::ConstantHeader);
    let has_config_resolution = context.shared.config_resolution.is_some();
    let config_resolution_context = config_constant_context && has_config_resolution;
    let source_build_config_contract_name = path_fork
        .component(qualified_name)
        .and_then(|name| BuildInputName::new(string_table.resolve(name)).ok());
    let has_source_build_config_contract = context
        .shared
        .source_build_config_contract_names
        .as_ref()
        .is_some_and(|names| {
            source_build_config_contract_name
                .as_ref()
                .is_some_and(|name| names.contains(name))
        });
    let source_build_config_context = config_constant_context && has_source_build_config_contract;
    if let Some(qualifier) = &declaration_syntax.config_qualifier
        && !config_resolution_context
        && !source_build_config_context
    {
        let mut diagnostic = CompilerDiagnostic::invalid_config_reason(
            path_fork.component(qualified_name),
            InvalidConfigReason::ConfigQualifierInvalidPlacement,
            qualifier.qualifier_span,
        );
        diagnostic.primary_span = qualifier.qualifier_span;
        return Err(diagnostic.into());
    }

    // ----------------------------
    //  Validate constant-context constraints
    // ----------------------------
    let is_reactive_binding = declaration_syntax.binding_mode.is_reactive();
    let value_mode = declaration_syntax.value_mode();
    if declaration_syntax.binding_mode.is_mutable() && context.kind.is_constant_context() {
        return Err(CompilerDiagnostic::invalid_declaration(
            InvalidDeclarationReason::ConstantCannotBeMutable,
            None,
            declaration_syntax.span,
        )
        .into());
    }

    // ----------------------------
    //  Resolve declared type
    // ----------------------------

    if let Some(capacity) = capacity_only_shorthand(&declaration_syntax.type_annotation) {
        let folded_capacity = fold_collection_capacity(
            capacity,
            Some(&*context),
            type_interner.environment_mut_for_derived_types(),
        )
        .map_err(|diagnostic| diagnostic.into_diagnostic())?;

        let mut initializer_stream = declaration_initializer_stream(
            source_owner,
            declaration_syntax.initializer_range,
            declaration_syntax.span,
        )?;

        // Shorthand requires an immediate collection literal initializer.
        if initializer_stream.current_tag() != TokenTag::OPEN_CURLY {
            return Err(CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
                Some(initializer_stream.current_span()),
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
            path_fork,
        )?;

        // The collection literal parser stops at the closing `}` without consuming it.
        // Advance past it so the remainder of the declaration validation sees EOF.
        if initializer_stream.current_tag() == TokenTag::CLOSE_CURLY {
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
                path_fork.component(qualified_name),
                declaration_syntax.span,
            )
            .into());
        }

        initializer_stream.skip_newlines();
        if initializer_stream.current_tag() != TokenTag::EOF {
            let found = initializer_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "declaration initializer diagnostic",
                    )
                })?
                .unwrap_or_else(|| {
                    DiagnosticToken::from_static_tag(initializer_stream.current_tag())
                });
            return Err(CompilerDiagnostic::unexpected_token_from_tag(
                found,
                Some(initializer_stream.current_span()),
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
            binding_span: None,
            config_qualifier,
        });
    }

    let resolved_annotation = {
        let mut type_resolution_context =
            TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
                declaration_table: &context.top_level_declarations,
                declaring_file_id: context.shared.declaring_file_id,
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
            declaration_syntax.span,
            &mut type_resolution_context,
            string_table,
            Some(context),
        )?
    };
    // Declaration-boundary config bootstrap permits required and optional contracts to omit `=`.
    // The config owner validates the contract and supplies a provider/default before the ordinary
    // constant fold; do not force a second initializer parser or invent a placeholder value here.
    if config_resolution_context
        && declaration_syntax.config_qualifier.is_some()
        && declaration_syntax.initializer_range.is_none()
    {
        return Ok(Declaration {
            id: qualified_name,
            value: Expression::no_value(
                declaration_syntax.span,
                DataType::Inferred,
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier,
        });
    }
    if source_build_config_context && declaration_syntax.config_qualifier.is_some() {
        let name = path_fork.component(qualified_name).ok_or_else(|| {
            CompilerError::compiler_error(
                "source #Config declaration has no terminal declaration name",
            )
        })?;
        let name_text = string_table.resolve(name);
        let input_name = BuildInputName::new(name_text).map_err(|_| {
            CompilerError::compiler_error(format!(
                "source #Config declaration has invalid build-input name '{name_text}'"
            ))
        })?;
        let resolved = context
            .shared
            .source_build_config_values
            .as_ref()
            .and_then(|values| values.get(&input_name))
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "source #Config declaration '{name_text}' has no resolved boundary value"
                ))
            })?;
        let value = expression_for_resolved_build_config_value(
            resolved,
            declaration_syntax.config_qualifier.as_ref().map_or_else(
                || declaration_syntax.span,
                |qualifier| qualifier.qualifier_span,
            ),
            type_interner,
            string_table,
        );
        return Ok(Declaration {
            id: qualified_name,
            value,
            binding_span: None,
            config_qualifier,
        });
    }
    let mut initializer_stream = declaration_initializer_stream(
        source_owner,
        declaration_syntax.initializer_range,
        declaration_syntax.span,
    )?;

    let mut parsed_initializer = match initializer_stream.current_tag() {
        // Struct Definition
        //
        // Compile-time `| name = expr |` and empty `#= | |` are const records. Ordinary
        // empty `| |` and `| name Type |` stay with the struct shell grammar.
        TokenTag::TYPE_PARAMETER_BRACKET
            if !initializer_stream
                .declaration_cursor()
                .map(|cursor| {
                    pipe_opens_value_record_at_cursor(
                        &cursor,
                        cursor.position(),
                        declaration_syntax.binding_mode.is_compile_time(),
                    )
                })
                .unwrap_or(false) =>
        {
            // Struct field defaults must be compile-time foldable, so they are parsed
            // in a dedicated constant context.
            let constant_context = ScopeContext::new_constant(qualified_name, context);
            let mut field_warnings = Vec::new();
            let field_syntax = {
                let (field_syntax, next_index) = {
                    let mut declaration_cursor = initializer_stream.declaration_cursor()?;
                    let field_syntax = parse_struct_shell(
                        &mut declaration_cursor,
                        string_table,
                        &mut field_warnings,
                        qualified_name,
                        path_fork,
                        &mut span_builder,
                    )?;
                    (field_syntax, declaration_cursor.position())
                };
                initializer_stream.set_position(next_index)?;
                field_syntax
            };
            for warning in field_warnings {
                context.emit_warning(warning);
            }
            let source_owner = source_owner.ok_or_else(|| {
                CompilerError::compiler_error(
                    "struct field defaults have no canonical source token owner",
                )
            })?;

            let mut params = Vec::with_capacity(field_syntax.len());
            for field in &field_syntax {
                params.push(signature_member_to_declaration(
                    field,
                    source_owner,
                    &constant_context,
                    type_interner,
                    string_table,
                    SignatureTypeFallbackPolicy::StrictCapacity,
                    path_fork,
                )?);
            }

            validate_struct_default_values(&params, &context.template_ir_store)
                .map_err(ExpressionParseError::from)?;

            Expression::struct_definition(params, declaration_syntax.span, value_mode.to_owned())
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
                        &*path_fork,
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
                path_fork,
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
                        path_fork,
                    },
                    false,
                );
                create_expression_with_trailing_newline_policy(input)?
            };

            if config_resolution_context && declaration_syntax.config_qualifier.is_some() {
                // Config bootstrap validates the authored fallback itself. Do not let the
                // declaration's explicit annotation coerce or reject the fallback first, because
                // an explicit provider must not mask a malformed authored default.
                expression
            } else if let Some(declared_type_id) = declared_type_id {
                // This is an explicit typed boundary: apply ordinary contextual coercions in
                // one shared path.
                coerce_expression_to_explicit_type_boundary(
                    expression,
                    declared_type_id,
                    type_interner.environment(),
                    TypeMismatchContext::Declaration,
                )?
            } else {
                expression
            }
        }
    };

    if !config_resolution_context {
        reject_config_qualifiers_on_record_fields(&parsed_initializer, path_fork)?;
    }

    // Body-local compile-time constants must fully fold after parsing and coercion.
    let initializer_is_compile_time_constant = if context.shared.config_resolution.is_some() {
        initializer_is_compile_time_constant_with_config_placeholders(&parsed_initializer, context)?
    } else {
        initializer_is_compile_time_constant(&parsed_initializer, context)?
    };

    if declaration_syntax.binding_mode.is_compile_time() && !initializer_is_compile_time_constant {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
            path_fork.component(qualified_name),
            declaration_syntax.span,
        )
        .into());
    }

    // Defensive: ensure the initializer parser consumed all tokens.
    // If tokens remain, the parser stopped early (e.g. a newline broke the
    // expression before it was complete). This prevents silent truncation.
    initializer_stream.skip_newlines();
    if initializer_stream.current_tag() == TokenTag::ELSE
        && type_interner
            .environment()
            .option_inner_type(parsed_initializer.type_id)
            .is_some()
    {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::DirectOptionFallbackSyntax,
            Some(initializer_stream.current_span()),
        )
        .into());
    }

    if initializer_stream.current_tag() != TokenTag::EOF {
        if initializer_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET
            && matches!(
                parsed_initializer.kind,
                ExpressionKind::AnonymousConstRecord { .. }
            )
        {
            // Extra `|` after a complete record is the usual leftover from an
            // inline nested `|...|` that the parser already closed.
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::NestedAnonymousConstRecord,
                Some(initializer_stream.current_span()),
            )
            .into());
        }

        let found = initializer_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "declaration initializer diagnostic",
                )
            })?
            .unwrap_or_else(|| DiagnosticToken::from_static_tag(initializer_stream.current_tag()));
        return Err(CompilerDiagnostic::unexpected_token_from_tag(
            found,
            Some(initializer_stream.current_span()),
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
                parsed_initializer.span,
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

    ast_log!("Created new ", Cyan #value_mode);
    Ok(Declaration {
        id: qualified_name,
        value: parsed_initializer,
        binding_span: None,
        config_qualifier,
    })
}
/// Build the bounded parser cursor for a declaration initializer.
///
/// Source-owned ranges stay on the canonical `AstCursor` view. A missing owner is an
/// invariant failure: every declaration initializer is parsed through a nested canonical cursor.
fn declaration_initializer_stream<'tokens>(
    source_owner: Option<&AstCursor<'tokens>>,
    initializer_range: Option<TokenRange>,
    declaration_span: Option<SourceSpan>,
) -> DeclarationResult<AstCursor<'tokens>> {
    let range = initializer_range
        .ok_or_else(|| CompilerError::compiler_error("declaration initializer range is missing"))?;
    let eof_span = declaration_span
        .map(SourceSpan::local)
        .unwrap_or_else(LocalSpan::source_start);

    let source_owner = source_owner.ok_or_else(|| {
        CompilerError::compiler_error("declaration initializer has no canonical source owner")
    })?;
    let cursor = source_owner.nested_cursor(range).map_err(|error| {
        ExpressionParseError::from(CompilerError::compiler_error(format!(
            "declaration initializer range cursor construction failed: {error:?}"
        )))
    })?;
    let source_id = cursor.source_id();
    Ok(cursor.with_synthetic_eof_span(SourceSpan::new(source_id, eof_span)))
}
#[cfg(test)]
#[path = "tests/declaration_tests.rs"]
mod declaration_tests;

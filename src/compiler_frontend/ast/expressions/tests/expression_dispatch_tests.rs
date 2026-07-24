//! Expression token and identifier dispatch tests.
//!
//! WHAT: validates token steps and identifier-led semantic routing at the expression entry gate.
//! WHY: dispatch owns both token advancement and context-sensitive reference validation, so focused
//!      tests keep those decisions aligned without exercising unrelated statement parsing.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, Operator,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::expressions::parse_expression_dispatch::{
    ExpressionDispatchState, ExpressionTokenStep, dispatch_expression_token,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, InvalidExpressionReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceLocation, Token, TokenKind, TokenizerEntryMode,
};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn test_scope(string_table: &mut StringTable) -> (InternedPath, ScopeContext) {
    let scope = InternedPath::from_single_str("test.moth", string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    (scope, context)
}

fn numeric_token_kind(value: &str, string_table: &mut StringTable) -> TokenKind {
    TokenKind::NumericLiteral(NumericLiteralToken::test_new(value, string_table))
}

fn numeric_token(value: &str, scope: &InternedPath, string_table: &mut StringTable) -> Token {
    Token::new(
        TokenKind::NumericLiteral(NumericLiteralToken::test_new(value, string_table)),
        SourceLocation::new(scope.clone(), Default::default(), Default::default()),
    )
}

fn token(kind: TokenKind, scope: &InternedPath) -> Token {
    Token::new(
        kind,
        SourceLocation::new(scope.clone(), Default::default(), Default::default()),
    )
}

#[test]
fn hash_in_expression_position_rejected() {
    let mut string_table = StringTable::default();
    let (scope, context) = test_scope(&mut string_table);
    let tokens = vec![
        numeric_token("1", &scope, &mut string_table),
        token(TokenKind::Hash, &scope),
        numeric_token("2", &scope, &mut string_table),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope.clone(), tokens);
    let mut expression = vec![];
    let mut expected_type = ExpectedType::Infer;
    let mut next_number_negative = false;
    let mut state = ExpressionDispatchState {
        expected_type: &mut expected_type,
        cast_target_context: &mut CastTargetContext::None,
        value_mode: &ValueMode::ImmutableOwned,
        consume_closing_parenthesis: false,
        allow_boundary_catch: true,
        allow_expected_result_evidence: true,
        expression: &mut expression,
        next_number_negative: &mut next_number_negative,
    };
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    // First token: NumericLiteral(1)
    let result = dispatch_expression_token(
        numeric_token_kind("1", &mut string_table),
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
    );
    assert!(result.is_ok());
    stream.advance();

    // Second token: Hash — should error because next token is NumericLiteral, not TemplateHead
    let result = dispatch_expression_token(
        TokenKind::Hash,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
    );
    assert!(
        result.is_err(),
        "Expected Hash in expression position to be rejected"
    );
}

#[test]
fn hash_before_template_head_allowed() {
    let mut string_table = StringTable::default();
    let (scope, context) = test_scope(&mut string_table);
    let tokens = vec![
        token(TokenKind::Hash, &scope),
        token(TokenKind::TemplateHead, &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope.clone(), tokens);
    let mut expression = vec![];
    let mut expected_type = ExpectedType::Infer;
    let mut next_number_negative = false;
    let mut state = ExpressionDispatchState {
        expected_type: &mut expected_type,
        cast_target_context: &mut CastTargetContext::None,
        value_mode: &ValueMode::ImmutableOwned,
        consume_closing_parenthesis: false,
        allow_boundary_catch: true,
        allow_expected_result_evidence: true,
        expression: &mut expression,
        next_number_negative: &mut next_number_negative,
    };
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let result = dispatch_expression_token(
        TokenKind::Hash,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
    );
    assert!(
        result.is_ok(),
        "Expected Hash before TemplateHead to advance"
    );
}

#[test]
fn negative_token_before_identifier_pushes_unary_negation_operator() {
    let mut string_table = StringTable::default();
    let (scope, context) = test_scope(&mut string_table);
    let name = string_table.intern("count");
    let tokens = vec![
        token(TokenKind::Negative, &scope),
        token(TokenKind::Symbol(name), &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope.clone(), tokens);
    let mut expression = vec![];
    let mut expected_type = ExpectedType::Infer;
    let mut next_number_negative = false;
    let mut state = ExpressionDispatchState {
        expected_type: &mut expected_type,
        cast_target_context: &mut CastTargetContext::None,
        value_mode: &ValueMode::ImmutableOwned,
        consume_closing_parenthesis: false,
        allow_boundary_catch: true,
        allow_expected_result_evidence: true,
        expression: &mut expression,
        next_number_negative: &mut next_number_negative,
    };
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let result = dispatch_expression_token(
        TokenKind::Negative,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
    );

    assert!(matches!(result, Ok(ExpressionTokenStep::Advance)));
    assert!(!*state.next_number_negative);
    assert!(matches!(
        state.expression.first(),
        Some(ExpressionRpnItem::Operator {
            operator: Operator::Negate,
            ..
        })
    ));
}

#[test]
fn hash_from_tokenized_source_rejected() {
    let mut string_table = StringTable::default();
    let source = "result = 1 # 2";
    let file_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let file_tokens = tokenize(
        source,
        &file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .unwrap();

    // Find the tokens after "result = "
    let mut index = 0;
    while index < file_tokens.length {
        if matches!(file_tokens.tokens[index].kind, TokenKind::Assign) {
            index += 1;
            break;
        }
        index += 1;
    }

    // Slice from after Assign to end
    let expr_tokens: Vec<Token> = file_tokens.tokens[index..].to_vec();
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);
    let mut stream = FileTokens::new(scope.clone(), expr_tokens);

    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut expected_type = ExpectedType::Infer;

    let result = create_expression(
        &mut stream,
        &context,
        &mut type_interner,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "Expected expression with stray # to fail, but got: ok"
    );
}

use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

#[test]
fn full_frontend_stray_hash_error() {
    let source = "result = 1 # 2";
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));
}

#[test]
fn constant_identifier_uses_module_store_tir() {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);
    let constant_name = string_table.intern("wrapper");

    let store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let location = SourceLocation::new(scope.clone(), Default::default(), Default::default());
    let template_id = {
        let mut store = store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node(SlotKey::Default, location.clone());
        builder.finish_template(
            slot,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location.clone(),
        )
    };

    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location: location.clone(),
    };

    let mut context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_template_ir_store(Rc::clone(&store));
    context.set_local_declarations(vec![Declaration {
        id: InternedPath::from_components(vec![constant_name]),
        value: Expression::template(template, ValueMode::ImmutableOwned),
    }]);

    let tokens = vec![
        token(TokenKind::Symbol(constant_name), &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut token_stream = FileTokens::new(scope, tokens);
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut expected_type = ExpectedType::Infer;

    let parsed = create_expression(
        &mut token_stream,
        &context,
        &mut type_interner,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
    )
    .expect("module-store constant reference should inline through effective TIR");

    let ExpressionKind::Template(parsed_template) = parsed.kind else {
        panic!("constant reference should preserve the existing inlined template behavior");
    };
    assert_eq!(parsed_template.tir_reference.root, template_id);
}

// ----------------------------------
//  Adjacent operand source locations
// ----------------------------------

/// Converts a byte offset to the lexer's 1-indexed source column.
fn one_indexed_column(byte_index: usize) -> i32 {
    (byte_index as i32).saturating_add(1)
}

fn assert_adjacent_operand_reason(diagnostic: &CompilerDiagnostic) {
    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidExpression {
                reason: InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            }
        ),
        "adjacent operands must use the structured missing-operator reason, got {:?}",
        diagnostic.payload,
    );
}

#[test]
fn adjacent_numeric_literals_report_missing_operator_at_second_expression() {
    let source = "value = 1 2";
    let second_expression_column = one_indexed_column(
        source
            .rfind('2')
            .expect("source must contain second operand"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_expression_column,
        "primary location must start at the second expression, not the first operand",
    );
}

#[test]
fn adjacent_grouped_expression_reports_missing_operator_at_second_group() {
    let source = "value = (1) (2)";
    let second_group_column = one_indexed_column(
        source
            .rfind('(')
            .expect("source must contain a second group opening"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_group_column,
        "primary location must start at the second group opening, the second operand",
    );
}

#[test]
fn operand_before_template_reports_missing_operator_at_template_start() {
    let source = "value = 1 [: two]";
    let template_start_column = one_indexed_column(
        source
            .find("[:")
            .expect("source must contain template start"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, template_start_column,
        "primary location must start at the template, the second operand",
    );
}

#[test]
fn template_before_operand_reports_missing_operator_at_second_expression() {
    let source = "value = [: one] 2";
    let second_expression_column = one_indexed_column(
        source
            .rfind('2')
            .expect("source must contain second operand"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_expression_column,
        "primary location must start at the literal after the template",
    );
}

#[test]
fn comment_template_after_operand_stays_valid() {
    let _ = parse_single_file_ast("value = 1 [$note:ignored]");
}

#[test]
fn standalone_template_stays_valid() {
    let _ = parse_single_file_ast("value = [: one]");
}

#[test]
fn binary_expression_with_operator_between_operands_stays_valid() {
    let _ = parse_single_file_ast("value = 1 + 2");
}

#[test]
fn adjacent_identifier_reports_missing_operator_before_unknown_name_lookup() {
    let source = "value = 1 missing_name";
    let second_expression_column = one_indexed_column(
        source
            .find("missing_name")
            .expect("source must contain the second operand"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_expression_column,
        "primary location must start at the second identifier, not the first operand",
    );
}

#[test]
fn adjacent_symbol_led_call_reports_missing_operator_at_identifier_start() {
    let source = "value = 1 identity(2)";
    let identifier_column = one_indexed_column(
        source
            .find("identity")
            .expect("source must contain the call identifier"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, identifier_column,
        "primary location must start at the call identifier, not the completed call",
    );
}

#[test]
fn adjacent_value_templates_report_missing_operator_at_second_template() {
    let source = "value = [: one] [: two]";
    let second_template_column = one_indexed_column(
        source
            .rfind("[:")
            .expect("source must contain a second template"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_template_column,
        "primary location must start at the second template opening",
    );
}

#[test]
fn adjacent_curly_literal_reports_missing_operator_at_second_operand() {
    let source = "value = 1 {1}";
    let second_expression_column = one_indexed_column(
        source
            .rfind('{')
            .expect("source must contain a curly literal opening"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_expression_column,
        "primary location must start at the curly literal opening, the second operand",
    );
}

#[test]
fn adjacent_copy_reports_missing_operator_at_copy_keyword() {
    let source = "value = 1 copy place";
    let second_expression_column = one_indexed_column(
        source
            .find("copy")
            .expect("source must contain the copy keyword"),
    );
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert_adjacent_operand_reason(&diagnostic);
    assert_eq!(
        diagnostic.primary_location.start_pos.char_column, second_expression_column,
        "primary location must start at the copy keyword, the second operand",
    );
}

#[test]
fn value_template_followed_by_comment_template_stays_valid() {
    let _ = parse_single_file_ast("value = [: one] [$note:ignored]");
}

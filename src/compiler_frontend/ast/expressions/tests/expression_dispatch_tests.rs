//! Expression token and identifier dispatch tests.
//!
//! WHAT: validates token steps and identifier-led semantic routing at the expression entry gate.
//! WHY: dispatch owns both token advancement and context-sensitive reference validation, so focused
//!      tests keep those decisions aligned without exercising unrelated statement parsing.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::cursor::AstCursor;
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
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    TestSourceTokensBuilder, TokenTag, TokenizerEntryMode,
};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn test_scope(string_table: &mut StringTable) -> (PathId, ScopeContext, PathInternerFork) {
    let mut path_fork = PathInternerFork::empty();
    let scope = path_fork
        .try_intern_portable_path("test.moth", string_table)
        .expect("test path fits");
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    (scope, context, path_fork)
}

fn numeric_token(
    builder: &mut TestSourceTokensBuilder,
    value: &str,
    string_table: &mut StringTable,
) {
    builder
        .push_numeric(
            NumericLiteralToken::test_new(value, string_table),
            LocalSpan::source_start(),
        )
        .expect("numeric fixture token should build");
}

fn static_token(builder: &mut TestSourceTokensBuilder, tag: TokenTag) {
    builder
        .push_static(tag, LocalSpan::source_start())
        .expect("static fixture token should build");
}

#[test]
fn hash_in_expression_position_rejected() {
    let mut string_table = StringTable::default();
    let (scope, context, mut path_fork) = test_scope(&mut string_table);
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    numeric_token(&mut builder, "1", &mut string_table);
    static_token(&mut builder, TokenTag::HASH);
    numeric_token(&mut builder, "2", &mut string_table);
    static_token(&mut builder, TokenTag::EOF);
    let owner = builder.finish().expect("canonical fixture tokens should build");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, None, range)
        .expect("test token stream must expose an AST cursor");
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
        TokenTag::NUMERIC_LITERAL,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_ok());
    stream.advance();

    // Second token: Hash — should error because next token is NumericLiteral, not TemplateHead
    let result = dispatch_expression_token(
        TokenTag::HASH,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
        &mut path_fork,
    );
    assert!(
        result.is_err(),
        "Expected Hash in expression position to be rejected"
    );
}

#[test]
fn hash_before_template_head_allowed() {
    let mut string_table = StringTable::default();
    let (scope, context, mut path_fork) = test_scope(&mut string_table);
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    static_token(&mut builder, TokenTag::HASH);
    static_token(&mut builder, TokenTag::TEMPLATE_HEAD);
    static_token(&mut builder, TokenTag::EOF);
    let owner = builder.finish().expect("canonical fixture tokens should build");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, None, range)
        .expect("test token stream must expose an AST cursor");
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
        TokenTag::HASH,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
        &mut path_fork,
    );
    assert!(
        result.is_ok(),
        "Expected Hash before TemplateHead to advance"
    );
}

#[test]
fn negative_token_before_identifier_pushes_unary_negation_operator() {
    let mut string_table = StringTable::default();
    let (scope, context, mut path_fork) = test_scope(&mut string_table);
    let name = string_table.intern("count");
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    static_token(&mut builder, TokenTag::NEGATIVE);
    builder
        .push_symbol(TokenTag::SYMBOL, name, LocalSpan::source_start())
        .expect("symbol fixture token should build");
    static_token(&mut builder, TokenTag::EOF);
    let owner = builder.finish().expect("canonical fixture tokens should build");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, None, range)
        .expect("test token stream must expose an AST cursor");
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
        TokenTag::NEGATIVE,
        &mut stream,
        &context,
        &mut type_interner,
        &mut state,
        &mut string_table,
        &mut path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::default();
    let source = "result = 1 # 2";
    let file_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .unwrap();
    let range = lexed
        .tokens
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&lexed.tokens, None, range)
        .expect("test token stream must expose an AST cursor");
    while stream.current_tag() != TokenTag::ASSIGN {
        assert!(
            !stream.is_at_end(),
            "the source fixture must contain an assignment"
        );
        stream.advance();
    }
    stream.advance();

    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        file_path,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
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
        &mut path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let scope = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let constant_name = string_table.intern("wrapper");

    let store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let template_id = {
        let mut store = store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node(SlotKey::Default, None);
        builder.finish_template(
            slot,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    };

    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        span: None,
    };

    let mut context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_template_ir_store(Rc::clone(&store));
    context.set_local_declarations(
        vec![Declaration {
            id: path_fork
                .try_intern_components(&[constant_name])
                .expect("test path fits"),
            value: Expression::template(template, ValueMode::ImmutableOwned),
            binding_span: None,
            config_qualifier: None,
        }],
        &path_fork,
    );

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_symbol(
            TokenTag::SYMBOL,
            constant_name,
            LocalSpan::source_start(),
        )
        .expect("symbol fixture token should build");
    static_token(&mut builder, TokenTag::EOF);
    let owner = builder.finish().expect("canonical fixture tokens should build");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, None, range)
        .expect("test token stream must expose an AST cursor");
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
        &mut path_fork,
    )
    .expect("module-store constant reference should inline through effective TIR");

    let ExpressionKind::Template(parsed_template) = parsed.kind else {
        panic!("constant reference should preserve the existing inlined template behavior");
    };
    assert_eq!(parsed_template.tir_reference.root, template_id);
}

//  Adjacent operand spans
// ----------------------------------

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
    assert!(diagnostic.primary_span.is_some());
}

#[test]
fn adjacent_numeric_literals_report_missing_operator_at_second_expression() {
    let diagnostic = parse_single_file_ast_diagnostic("value = 1 2");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn adjacent_grouped_expression_reports_missing_operator_at_second_group() {
    let diagnostic = parse_single_file_ast_diagnostic("value = (1) (2)");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn operand_before_template_reports_missing_operator_at_template_start() {
    let diagnostic = parse_single_file_ast_diagnostic("value = 1 [: two]");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn template_before_operand_reports_missing_operator_at_second_expression() {
    let diagnostic = parse_single_file_ast_diagnostic("value = [: one] 2");
    assert_adjacent_operand_reason(&diagnostic);
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
    let diagnostic = parse_single_file_ast_diagnostic("value = 1 missing_name");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn adjacent_symbol_led_call_reports_missing_operator_at_identifier_start() {
    let diagnostic = parse_single_file_ast_diagnostic("value = 1 identity(2)");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn adjacent_value_templates_report_missing_operator_at_second_template() {
    let diagnostic = parse_single_file_ast_diagnostic("value = [: one] [: two]");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn adjacent_curly_literal_reports_missing_operator_at_second_operand() {
    let diagnostic = parse_single_file_ast_diagnostic("value = 1 {1}");
    assert_adjacent_operand_reason(&diagnostic);
}

#[test]
fn adjacent_copy_reports_missing_operator_at_copy_keyword() {
    let diagnostic = parse_single_file_ast_diagnostic("value = 1 copy place");
    assert_adjacent_operand_reason(&diagnostic);
}
#[test]
fn value_template_followed_by_comment_template_stays_valid() {
    let _ = parse_single_file_ast("value = [: one] [$note:ignored]");
}

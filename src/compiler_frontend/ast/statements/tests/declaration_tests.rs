//! Variable declaration parsing regression tests.
//!
//! WHAT: validates mutability, explicit types, and named-type annotations in declarations.
//! WHY: declaration parsing is the entrypoint for most AST values and must preserve type intent.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::ConstValuePayload;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::InvalidValueReceiverView,
    names_declarations::{ReservedNameCollisionView, ShadowedNameView},
    token_lexical::{ExpectedTokenView, UnexpectedTokenShape, UnexpectedTokenView},
    types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, ReservedNameOwner, TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::display::display_type_with_resolver;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_declaration_syntax;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalPackageRegistry, ExternalSymbolId,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span,
    ast_first_error_type_display, freeze_ast_diagnostic_failure, parse_single_file_ast,
    parse_single_file_ast_build_result_with_registry, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

// --------------------------
//  Basic declarations
// --------------------------

#[test]
fn parses_mutable_and_explicitly_typed_declarations() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("count ~= 1\nname String = \"Ada\"\n");

    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(count_decl) = &body[0].kind else {
        panic!("expected mutable declaration");
    };
    assert_eq!(
        count_decl.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_eq!(count_decl.value_mode, ValueMode::MutableOwned);

    let NodeKind::VariableDeclaration(name_decl) = &body[1].kind else {
        panic!("expected explicit string declaration");
    };
    assert_eq!(name_decl.type_id, builtin_type_ids::STRING);
    let initializer = ast
        .expressions
        .view(name_decl.value.initializer().expect("fixture initializer"))
        .expect("initializer belongs to AST");
    let ExpressionOperationView::Constant(value) = initializer.operation() else {
        panic!("expected folded string initializer");
    };
    assert!(matches!(
        ast.const_values.payload(value),
        Some(ConstValuePayload::String(_))
    ));
}

#[test]
fn resolves_named_type_annotations_against_prior_structs() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("Point = |\n    x I32,\n|\n\norigin Point = Point(0)\n");

    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(origin_decl) = &body[0].kind else {
        panic!("expected typed declaration");
    };
    assert!(matches!(
        ast.type_environment.get(origin_decl.type_id),
        Some(TypeDefinition::Struct(definition)) if !definition.const_record
    ));
    let initializer = ast
        .expressions
        .view(
            origin_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("initializer belongs to AST");
    match initializer.operation() {
        ExpressionOperationView::Struct { .. } => {}
        ExpressionOperationView::Constant(value) => {
            assert!(matches!(
                ast.const_values.payload(value),
                Some(ConstValuePayload::Record(_))
            ));
        }
        _ => panic!("expected struct initializer"),
    }
}

// --------------------------
//  Reserved name rejections
// --------------------------

#[test]
fn rejects_user_declarations_named_error() {
    let failure = parse_single_file_ast_diagnostic("Error = 1\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the diagnostic is a reserved-name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::BuiltinType);
    let (report, _) = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("the failure contains its first error report");
    assert_eq!(report.context().strings().resolve(facts.name), "Error");
}

#[test]
fn rejects_struct_redefinition_of_reserved_error_symbol() {
    let failure = parse_single_file_ast_diagnostic("Error = |\n    message String,\n|\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the diagnostic is a reserved-name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::BuiltinType);
    let (report, _) = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("the failure contains its first error report");
    assert_eq!(report.context().strings().resolve(facts.name), "Error");
}

#[test]
fn allows_user_declarations_named_old_error_family_symbols() {
    parse_single_file_ast(
        "ErrorKind = |\n    message String,\n|\n\n\
         ErrorLocation = |\n    line I32,\n|\n\n\
         StackFrame = |\n    name String,\n|\n\n\
         kind = ErrorKind(\"custom\")\n\
         location = ErrorLocation(12)\n\
         frame = StackFrame(\"main\")\n",
    );
}

#[test]
fn rejects_keyword_shadow_variable_declarations() {
    let failure = parse_single_file_ast_diagnostic("_true = 1\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the diagnostic is a reserved-name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::Keyword);
    let (report, _) = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("the failure contains its first error report");
    assert_eq!(report.context().strings().resolve(facts.name), "_true");
}

#[test]
fn shadowed_declaration_retains_exact_duplicate_name_span() {
    let source = "value = \"π\"\nvalue I32 = 2\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0038", None);

    let duplicate_start = source
        .rfind("value I32")
        .expect("the duplicate declaration should be present") as u32;
    let mut expected_builder = ExtendedSpanBuilder::new();
    let expected_span =
        LocalSpan::exact(duplicate_start, "value".len() as u32, &mut expected_builder)
            .expect("the duplicate name span should fit the inline representation");

    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(crate::compiler_frontend::source::SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            expected_span,
        ))
    );
    let labels = ast_error_label_spans(diagnostic);
    assert_eq!(labels.len(), 1);
    assert_eq!(
        labels[0],
        Some(crate::compiler_frontend::source::SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            LocalSpan::exact(0, "value".len() as u32, &mut expected_builder)
                .expect("the original name span should fit"),
        )),
    );
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(matches!(
        labels[0].message,
        DurableLabelMessage::PreviousDeclaration
    ));
}

fn body_prelude_registry() -> ExternalPackageRegistry {
    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_prelude_symbol(
            "prelude_print",
            ExternalSymbolId::Function(ExternalFunctionId::IoPrint),
        )
        .expect("the prelude function alias must register");
    registry
}

#[test]
fn body_local_prelude_collisions_retain_authored_name_span() {
    for (source, name) in [
        ("io = 1\n", "io"),
        ("io I32 = 1\n", "io"),
        ("prelude_print = 1\n", "prelude_print"),
        ("prelude_print I32 = 1\n", "prelude_print"),
        ("if true:\n    io = 1\n;\n", "io"),
        (
            "pair || -> I32, I32:\n    return 1, 2\n;\nvalue, io = pair()\n",
            "io",
        ),
        (
            "pair || -> I32, I32:\n    return 1, 2\n;\nvalue I32, io I32 = pair()\n",
            "io",
        ),
        (
            "@core/io line as host_line\nhost_line I32 = 1\n",
            "host_line",
        ),
        ("@core/io as output\noutput = 1\n", "output"),
        (
            "@core/io line as host_line\npair || -> I32, I32:\n    return 1, 2\n;\nvalue, host_line I32 = pair()\n",
            "host_line",
        ),
    ] {
        let failure =
            parse_single_file_ast_build_result_with_registry(source, body_prelude_registry())
                .err()
                .expect("a body binding must not replace a visible prelude binding");
        let failure = freeze_ast_diagnostic_failure(failure);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0038", None);
        let facts = diagnostic
            .family_view::<ShadowedNameView>()
            .expect("the diagnostic is a shadowed name");
        let (report, _) = failure
            .reports
            .iter_severity(DiagnosticSeverity::Error)
            .next()
            .expect("the failure contains its first error report");
        assert_eq!(
            report.context().strings().resolve(facts.name),
            name,
            "{source}"
        );
        let duplicate_start = source
            .rfind(name)
            .expect("the duplicate declaration name must be present")
            as u32;
        let mut expected_builder = ExtendedSpanBuilder::new();
        let expected_span =
            LocalSpan::exact(duplicate_start, name.len() as u32, &mut expected_builder)
                .expect("the duplicate name span must fit the inline representation");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, expected_span)),
            "{source}",
        );
        assert!(
            ast_error_label_spans(diagnostic).is_empty(),
            "{source}: prelude collisions have no secondary declaration label",
        );
        assert_eq!(
            failure.reports.iter().count(),
            1,
            "{source}: expected one shadowing diagnostic for {name}",
        );
    }
}

#[test]
fn body_local_prelude_lookup_preserves_namespace_and_function_calls() {
    let (result, paths, strings) = parse_single_file_ast_build_result_with_registry(
        "message = \"hello\"\nio.print(message)\nprelude_print(message)\n",
        body_prelude_registry(),
    )
    .expect("ordinary locals and original prelude calls must remain valid");
    let body = start_function_body(&result.ast, &paths, &strings);
    assert!(matches!(body[0].kind, NodeKind::VariableDeclaration(_)));
    for statement in &body[1..] {
        let NodeKind::ExpressionStatement(expression) = statement.kind else {
            panic!("the prelude use must remain an expression statement");
        };
        assert!(matches!(
            result.ast.expressions.view(expression).unwrap().operation(),
            ExpressionOperationView::Call(_)
        ));
    }
    assert_eq!(body.len(), 3);
}

#[test]
fn malformed_multi_bind_keeps_missing_target_priority_over_name_collision() {
    let source = "pair || -> I32, I32:\n    return 1, 2\n;\nio, = pair()\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0002",
        Some("invalid_multi_bind.missing_target_after_comma"),
    );
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("the diagnostic is an unexpected token");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            target_name: None,
            ..
        }
    ));
    let comma = source.rfind(',').expect("the target comma must occur") as u32;
    let mut builder = ExtendedSpanBuilder::new();
    let span = LocalSpan::exact(comma, 1, &mut builder).expect("the comma span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
    );
}

#[test]
fn body_local_prelude_malformed_host_statement_keeps_missing_parenthesis_diagnostic() {
    let source = "@core/io line as print\nprint\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0001", None);
    let facts = diagnostic
        .family_view::<ExpectedTokenView>()
        .expect("the diagnostic is an expected-token failure");
    assert_eq!(
        facts.expected,
        DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
    );
    assert_eq!(
        facts.found,
        Some(DiagnosticToken::from_static_tag(TokenTag::NEWLINE)),
    );
    let mut builder = ExtendedSpanBuilder::new();
    let span = LocalSpan::exact((source.len() - 1) as u32, 1, &mut builder)
        .expect("the newline span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
    );
}

#[test]
fn rejects_unconsumed_source_config_qualifier_at_declaration_boundary() {
    let failure = parse_single_file_ast_diagnostic("analytics #Config of Bool = false\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-CONFIG-0001",
        Some("invalid_config.config_qualifier_invalid_placement"),
    );
}

//  Type mismatch diagnostics
// --------------------------

#[test]
fn rejects_initializer_type_mismatch() {
    let failure = parse_single_file_ast_diagnostic("result F64 = true\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the diagnostic is a type mismatch");
    assert_eq!(facts.context, TypeMismatchContext::Declaration);
    assert_eq!(
        ast_first_error_type_display(&failure, facts.expected),
        "F64"
    );
    assert_eq!(ast_first_error_type_display(&failure, facts.found), "Bool");
}

#[test]
fn rejects_multiline_regular_division_with_operator_on_next_line() {
    assert_declaration_type_mismatch("result I32 = 5\n / 2\n");
}

#[test]
fn rejects_multiline_regular_division_with_operator_at_end_of_line() {
    assert_declaration_type_mismatch("result I32 = 5 /\n 2\n");
}

fn assert_declaration_type_mismatch(source: &str) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
}

// --------------------------
//  Capacity-only shorthand declarations
// --------------------------

#[test]
fn shorthand_fixed_collection_declaration_infers_element_type() {
    let (ast, path_fork, string_table) = parse_single_file_ast("items {2} = {1, 2}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 I32}"
    );
}

#[test]
fn fixed_collection_alias_literal_is_accepted() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        r#"
    Names as {2 String}
    names Names = {"Priya"}
    "#,
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 String}"
    );
}

#[test]
fn nested_fixed_collection_literal_is_accepted() {
    let (ast, path_fork, string_table) = parse_single_file_ast("grid {2 {3 I32}} = {{1}, {2}}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 {3 I32}}"
    );
}

#[test]
fn immutable_fixed_collection_from_function_call_is_allowed() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        r#"
    make || -> {2 I32}:
        return {1}
    ;
    
    items {2 I32} = make()
    "#,
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 I32}"
    );
}

#[test]
fn generic_identity_preserves_fixed_collection_shape() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        r#"
    identity type Item |value Item| -> Item:
        return value
    ;
    
    items {2 I32} = {1}
    same = identity(items)
    "#,
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[1].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 I32}"
    );
}

#[test]
fn fixed_collection_struct_field_default_allows_empty_literal() {
    parse_single_file_ast(
        r#"
Buffer = |
    items {2 I32} = {},
|

buffer ~= Buffer()
"#,
    );
}

#[test]
fn mutable_empty_fixed_collection_is_allowed() {
    let (ast, path_fork, string_table) = parse_single_file_ast("items ~{2 I32} = {}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 I32}"
    );
}

#[test]
fn normal_collection_declaration() {
    let (ast, path_fork, string_table) = parse_single_file_ast("items {I32} = {1, 2}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };
    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{I32}"
    );
}

#[test]
fn fixed_and_growable_assignment_mismatch_is_rejected() {
    let failure = parse_single_file_ast_diagnostic(
        r#"
growable {I32} = {1}
items {2 I32} = growable
"#,
    );
    assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
}

#[test]
fn exact_fixed_capacity_mismatch_is_rejected() {
    let failure = parse_single_file_ast_diagnostic(
        r#"
three {3 I32} = {1}
items {2 I32} = three
"#,
    );
    assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
}

#[test]
fn initializer_terminator_preserves_the_parsed_declaration_anchor() {
    let padding = "🦋".repeat(600);
    let long_type = "LongType".repeat(150);
    let long_target = format!("{long_type} = 1");
    for (target, expected_anchor) in [
        ("= 1", "="),
        ("I32 = 1", "I32"),
        ("~= 1", "~"),
        ("#= 1", "#"),
        ("#Config of I32 = 1", "#"),
        (long_target.as_str(), long_type.as_str()),
    ] {
        let source = format!("padding #= \"{padding}\"\nvalue {target}\n");
        let mut strings = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let source_path = path_fork
            .try_intern_portable_path("declarations.moth", &mut strings)
            .expect("test path fits");
        let canonical_path = PathBuf::from("declarations.moth");
        let mut sources =
            SourceDatabase::build([&canonical_path], &canonical_path, None, &mut strings)
                .expect("the authored source must register before tokenization");
        let file_id = sources.get_by_canonical_path(&canonical_path).unwrap().id;
        sources
            .retain_text(file_id, source)
            .expect("the original source snapshot must load");
        let source = sources.retained_text(file_id).unwrap();
        let mut builder = ExtendedSpanBuilder::new();
        let lexed = tokenize(
            source,
            source_path,
            TokenizerEntryMode::SourceFile,
            &StyleDirectiveRegistry::built_ins(),
            &mut strings,
            &mut path_fork,
            file_id,
            &mut builder,
        )
        .expect("the source must tokenize");
        let name = strings.intern("value");
        let declaration_position = (0..lexed.tokens.len())
            .find_map(|index| {
                let index = TokenIndex::try_from_index(index)?;
                let token = lexed.tokens.token(index).ok()?;
                (token.tag() == TokenTag::SYMBOL && token.string_id() == Some(name))
                    .then_some(index.index() + 1)
            })
            .expect("the tokenized source must contain the declaration name");
        let full_range = lexed
            .tokens
            .full_range()
            .expect("the tokenized source must expose a checked full range");
        let mut canonical_cursor = lexed
            .tokens
            .cursor(full_range)
            .expect("the tokenized source must expose canonical tokens");
        canonical_cursor
            .set_position(
                TokenIndex::try_from_index(declaration_position)
                    .expect("the declaration position must fit the canonical index domain"),
            )
            .expect("the canonical cursor must seek to the declaration position");
        let (declaration, declaration_position) = {
            let mut declaration_cursor =
                crate::compiler_frontend::declaration_syntax::DeclarationCursor::new(
                    canonical_cursor,
                )
                .expect("the tokenized source must expose canonical tokens");
            let declaration =
                parse_declaration_syntax(&mut declaration_cursor, name, &mut strings, &mut builder)
                    .expect("the authored declaration must produce its shell");
            let declaration_position = declaration_cursor.position();
            (declaration, declaration_position)
        };
        let expected_start = source.rfind("value ").unwrap() as u32 + "value ".len() as u32;
        let expected_range = (
            expected_start,
            expected_start + expected_anchor.len() as u32,
        );
        let declaration_span = declaration
            .span
            .expect("the authored declaration must retain its source span");
        assert_eq!(declaration_span.source(), file_id);
        let range = declaration_span.resolve_with(builder.resolver_for(file_id));
        assert_eq!(
            (range.start(), range.end()),
            expected_range,
            "target {target}"
        );
        assert!(
            !builder.is_empty(),
            "the preceding long literal must use the original span table"
        );

        let _context = ScopeContext::new_for_tests(
            ContextKind::Function,
            source_path,
            Rc::new(TopLevelDeclarationTable::new(
                vec![],
                &PathInternerFork::empty(),
            )),
            Arc::new(ExternalPackageRegistry::new()),
            0,
        )
        .with_declaring_file_id(file_id);

        let source_owner = &lexed.tokens;
        let source_range = source_owner
            .full_range()
            .expect("the tokenized source must expose a checked full range");
        let mut owner = AstCursor::from_source_tokens(source_owner, source_range)
            .expect("the tokenized source must expose an AST cursor");
        owner
            .set_position(declaration_position)
            .expect("the canonical cursor must seek to the declaration position");
        let mut initializer = super::declaration_initializer_stream(
            Some(&owner),
            declaration.initializer_range,
            declaration.span,
        )
        .expect("initializer must retain its declaration's source owner");
        assert_eq!(initializer.source_id(), file_id);
        let eof_position = initializer.length();
        initializer
            .set_position(eof_position)
            .expect("initializer EOF position must be within its range");
        assert_eq!(initializer.current_span(), declaration_span);
        sources
            .install_extended_spans(file_id, builder.freeze())
            .expect("the original table must install after the final span producer");
        let range = initializer.current_span().byte_range(&sources);
        assert_eq!(
            (range.start(), range.end()),
            expected_range,
            "target {target}"
        );
    }
}

#[test]
fn single_declarations_reject_zero_and_multiple_success_slots() {
    for (producer, produced_count) in [
        ("none_here ||:\n    return\n;\n", 0),
        (
            "none_here || -> I32, String:\n    return 1, \"one\"\n;\n",
            2,
        ),
    ] {
        for receiver in ["value = none_here()", "value I32 = none_here()"] {
            let source = format!("{producer}\n{receiver}\n");
            let failure = parse_single_file_ast_diagnostic(&source);
            let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
            assert!(ast_error_primary_span(diagnostic).is_some(), "{source}");
            let facts = diagnostic
                .family_view::<InvalidValueReceiverView>()
                .expect("the diagnostic is an invalid value receiver");
            assert_eq!(
                facts.receiver,
                ValueReceiverContext::Declaration,
                "{source}"
            );
            assert_eq!(facts.target_count, 1, "{source}");
            assert_eq!(facts.produced_count, produced_count, "{source}");
        }
    }
}

#[test]
fn assignments_reject_zero_and_multiple_success_slots() {
    for (producer, produced_count) in [
        ("produce ||:\n    return\n;\n", 0),
        ("produce || -> I32, String:\n    return 1, \"one\"\n;\n", 2),
    ] {
        let source = format!("{producer}\nvalue ~= 0\nvalue = produce()\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
        let facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the diagnostic is an invalid value receiver");
        assert_eq!(facts.receiver, ValueReceiverContext::Assignment, "{source}");
        assert_eq!(facts.target_count, 1, "{source}");
        assert_eq!(facts.produced_count, produced_count, "{source}");
    }
}

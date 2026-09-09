//! Variable declaration parsing regression tests.
//!
//! WHAT: validates mutability, explicit types, and named-type annotations in declarations.
//! WHY: declaration parsing is the entrypoint for most AST values and must preserve type intent.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticLabelMessage, DiagnosticPayload, ReservedNameOwner,
    RuleDiagnosticKind, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_declaration_syntax;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenizerEntryMode};
use std::rc::Rc;
use std::sync::Arc;

use crate::compiler_frontend::value_mode::ValueMode;

// --------------------------
//  Basic declarations
// --------------------------

#[test]
fn parses_mutable_and_explicitly_typed_declarations() {
    let (ast, string_table) = parse_single_file_ast("count ~= 1\nname String = \"Ada\"\n");

    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(count_decl) = &body[0].kind else {
        panic!("expected mutable declaration");
    };
    assert_eq!(count_decl.value.diagnostic_type, DataType::Int);
    assert_eq!(count_decl.value.value_mode, ValueMode::MutableOwned);

    let NodeKind::VariableDeclaration(name_decl) = &body[1].kind else {
        panic!("expected explicit string declaration");
    };
    assert_eq!(name_decl.value.diagnostic_type, DataType::StringSlice);
    assert!(matches!(
        name_decl.value.kind,
        ExpressionKind::StringSlice(..)
    ));
}

#[test]
fn resolves_named_type_annotations_against_prior_structs() {
    let (ast, string_table) =
        parse_single_file_ast("Point = |\n    x Int,\n|\n\norigin Point = Point(0)\n");

    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(origin_decl) = &body[0].kind else {
        panic!("expected typed declaration");
    };
    assert!(matches!(
        origin_decl.value.diagnostic_type,
        DataType::Struct {
            const_record: false,
            ..
        }
    ));
    assert!(matches!(
        origin_decl.value.kind,
        ExpressionKind::StructInstance(..)
    ));
}

// --------------------------
//  Reserved name rejections
// --------------------------

#[test]
fn rejects_user_declarations_named_error() {
    let diagnostic = parse_single_file_ast_diagnostic("Error = 1\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::BuiltinType,
            ..
        }
    ));
}

#[test]
fn rejects_struct_redefinition_of_reserved_error_symbol() {
    let diagnostic = parse_single_file_ast_diagnostic("Error = |\n    message String,\n|\n");

    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName)
    ));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnusedName { .. }
    ));
}

#[test]
fn allows_user_declarations_named_old_error_family_symbols() {
    parse_single_file_ast(
        "ErrorKind = |\n    message String,\n|\n\n\
         ErrorLocation = |\n    line Int,\n|\n\n\
         StackFrame = |\n    name String,\n|\n\n\
         kind = ErrorKind(\"custom\")\n\
         location = ErrorLocation(12)\n\
         frame = StackFrame(\"main\")\n",
    );
}

#[test]
fn rejects_keyword_shadow_variable_declarations() {
    let diagnostic = parse_single_file_ast_diagnostic("_true = 1\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::Keyword,
            ..
        }
    ));
}

#[test]
fn shadowed_declaration_retains_exact_duplicate_name_span() {
    let source = "value = \"π\"\nvalue Int = 2\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::ShadowedName { name: _ }
    ));

    let duplicate_start = source
        .rfind("value Int")
        .expect("the duplicate declaration should be present") as u32;
    let mut expected_builder = ExtendedSpanBuilder::new();
    let expected_span =
        LocalSpan::exact(duplicate_start, "value".len() as u32, &mut expected_builder)
            .expect("the duplicate name span should fit the inline representation");

    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, expected_span))
    );
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::PreviousDeclaration)
    );
}

#[test]
fn rejects_unconsumed_source_config_qualifier_at_declaration_boundary() {
    let diagnostic = parse_single_file_ast_diagnostic("analytics #Config of Bool = false\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: crate::compiler_frontend::compiler_messages::InvalidConfigReason::ConfigQualifierInvalidPlacement,
            ..
        }
    ));
}

#[test]
fn rejects_config_qualifier_fields_in_source_constant_records() {
    let diagnostic =
        parse_single_file_ast_diagnostic("settings #= | flag #Config of Bool = false |\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: crate::compiler_frontend::compiler_messages::InvalidConfigReason::ConfigQualifierInvalidPlacement,
            ..
        }
    ));
}

//  Type mismatch diagnostics
// --------------------------

#[test]
fn rejects_initializer_type_mismatch_with_target_and_value_details() {
    let diagnostic = parse_single_file_ast_diagnostic("result Float = true\n");

    let DiagnosticPayload::TypeMismatch {
        expected,
        found,
        context,
    } = &diagnostic.payload
    else {
        panic!("expected typed TypeMismatch diagnostic payload");
    };

    assert_eq!(*context, TypeMismatchContext::Declaration);
    assert_eq!(expected.0, 2);
    assert_eq!(found.0, 0);
}

#[test]
fn rejects_multiline_regular_division_with_operator_on_next_line() {
    assert_declaration_type_mismatch("result Int = 5\n / 2\n");
}

#[test]
fn rejects_multiline_regular_division_with_operator_at_end_of_line() {
    assert_declaration_type_mismatch("result Int = 5 /\n 2\n");
}

fn assert_declaration_type_mismatch(source: &str) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(
        matches!(diagnostic.payload, DiagnosticPayload::TypeMismatch { .. }),
        "expected TypeMismatch, got {:?}",
        diagnostic.payload
    );
}

// --------------------------
//  Capacity-only shorthand declarations
// --------------------------

#[test]
fn shorthand_fixed_collection_declaration_infers_element_type() {
    let (ast, string_table) = parse_single_file_ast("items {2} = {1, 2}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 Int}"
    );
}

#[test]
fn fixed_collection_alias_literal_is_accepted() {
    let (ast, string_table) = parse_single_file_ast(
        r#"
Names as {2 String}
names Names = {"Priya"}
"#,
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 String}"
    );
}

#[test]
fn nested_fixed_collection_literal_is_accepted() {
    let (ast, string_table) = parse_single_file_ast("grid {2 {3 Int}} = {{1}, {2}}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 {3 Int}}"
    );
}

#[test]
fn immutable_fixed_collection_from_function_call_is_allowed() {
    let (ast, string_table) = parse_single_file_ast(
        r#"
make || -> {2 Int}:
    return {1}
;

items {2 Int} = make()
"#,
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 Int}"
    );
}

#[test]
fn generic_identity_preserves_fixed_collection_shape() {
    let (ast, string_table) = parse_single_file_ast(
        r#"
identity type Item |value Item| -> Item:
    return value
;

items {2 Int} = {1}
same = identity(items)
"#,
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[1].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 Int}"
    );
}

#[test]
fn fixed_collection_struct_field_default_allows_empty_literal() {
    parse_single_file_ast(
        r#"
Buffer = |
    items {2 Int} = {},
|

buffer ~= Buffer()
"#,
    );
}

#[test]
fn mutable_empty_fixed_collection_is_allowed() {
    let (ast, string_table) = parse_single_file_ast("items ~{2 Int} = {}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 Int}"
    );
}

#[test]
fn normal_collection_declaration() {
    let (ast, string_table) = parse_single_file_ast("items {Int} = {1, 2}\n");
    let body = start_function_body(&ast, &string_table);
    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };
    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{Int}"
    );
}

#[test]
fn fixed_and_growable_assignment_mismatch_is_rejected() {
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
growable {Int} = {1}
items {2 Int} = growable
"#,
    );

    assert!(
        matches!(diagnostic.payload, DiagnosticPayload::TypeMismatch { .. }),
        "expected TypeMismatch, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn exact_fixed_capacity_mismatch_is_rejected() {
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
three {3 Int} = {1}
items {2 Int} = three
"#,
    );

    assert!(
        matches!(diagnostic.payload, DiagnosticPayload::TypeMismatch { .. }),
        "expected TypeMismatch, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn initializer_terminator_preserves_the_parsed_declaration_anchor() {
    let padding = "🦋".repeat(600);
    let long_type = "LongType".repeat(150);
    let long_target = format!("{long_type} = 1");
    for (target, expected_anchor) in [
        ("= 1", "="),
        ("Int = 1", "Int"),
        ("~= 1", "~"),
        ("#= 1", "#"),
        ("#Config of Int = 1", "#"),
        (long_target.as_str(), long_type.as_str()),
    ] {
        let source = format!("padding #= \"{padding}\"\nvalue {target}\n");
        let mut strings = StringTable::new();
        let source_path = InternedPath::from_single_str("declarations.moth", &mut strings);
        let canonical_path = source_path.to_path_buf(&strings);
        let mut sources =
            SourceDatabase::build([&canonical_path], &canonical_path, None, &mut strings)
                .expect("the authored source must register before tokenization");
        let file_id = sources.get_by_canonical_path(&canonical_path).unwrap().id;
        sources
            .retain_text(file_id, source)
            .expect("the original source snapshot must load");
        let source = sources.retained_text(file_id).unwrap();
        let mut builder = ExtendedSpanBuilder::new();
        let mut tokens = tokenize(
            source,
            &source_path,
            TokenizerEntryMode::SourceFile,
            &StyleDirectiveRegistry::built_ins(),
            &mut strings,
            file_id,
            &mut builder,
        )
        .expect("the source must tokenize");
        let name = strings.intern("value");
        tokens.index = tokens
            .tokens
            .iter()
            .position(|token| token.kind == TokenKind::Symbol(name))
            .unwrap()
            + 1;
        let declaration = parse_declaration_syntax(&mut tokens, name, &mut strings, &mut builder)
            .expect("the authored declaration must produce its shell");
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

        tokens.freeze_path_syntax_for_test();
        let context = ScopeContext::new_for_tests(
            ContextKind::Function,
            source_path.clone(),
            Rc::new(TopLevelDeclarationTable::new(vec![])),
            Arc::new(ExternalPackageRegistry::new()),
            vec![],
            0,
        )
        .with_declaring_file_id(file_id);
        let initializer = super::declaration_initializer_stream(
            &source_path.append(name),
            declaration.initializer_tokens,
            &tokens.path_syntax,
            &context,
        )
        .expect("initializer must retain its declaration's source owner");
        let terminator = initializer.tokens.last().unwrap();
        assert_eq!(terminator.kind, TokenKind::Eof);
        assert_eq!(initializer.file_id, file_id);
        sources
            .install_extended_spans(file_id, builder.freeze())
            .expect("the original table must install after the final span producer");
        let range = SourceSpan::new(file_id, terminator.span).byte_range(&sources);
        assert_eq!(
            (range.start(), range.end()),
            expected_range,
            "target {target}"
        );
        let terminator_span = SourceSpan::new(file_id, terminator.span);
        assert_eq!(terminator_span, declaration_span);
    }
}

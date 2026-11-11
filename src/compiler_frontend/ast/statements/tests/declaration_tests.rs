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
use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::InvalidValueReceiverView,
    names_declarations::{DuplicateDeclarationView, ReservedNameCollisionView},
    token_lexical::{ExpectedTokenView, UnexpectedTokenShape, UnexpectedTokenView},
    types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticSeverity, DiagnosticToken, ReservedNameOwner, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::display::display_type_with_resolver;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_declaration_syntax;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalPackageRegistry, ExternalSymbolId,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceSpan,
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
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

#[test]
fn prefixed_statement_probe_respects_nested_syntax_and_boundaries() {
    for (statement, code, reason) in [
        (
            "~make().field = 1",
            "MOTH-RULE-0044",
            "invalid_assignment_target.mutable_marker_requires_existing_place",
        ),
        (
            "~(value) = 1",
            "MOTH-RULE-0044",
            "invalid_assignment_target.mutable_marker_requires_existing_place",
        ),
        (
            "~true\nnext = 1",
            "MOTH-RULE-0047",
            "invalid_receiver_call.mutable_marker_on_non_receiver_call",
        ),
    ] {
        let source = format!("value ~= 0\n{statement}\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, code, Some(reason));
        let mut spans = ExtendedSpanBuilder::new();
        let marker = LocalSpan::exact(source.find(statement).unwrap() as u32, 1, &mut spans)
            .expect("marker span fits");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, marker)),
            "{statement}",
        );
    }
}

#[test]
fn prefixed_receiver_calls_keep_nested_named_arguments_out_of_write_dispatch() {
    let source = "Point = |\n    x I32 = 0,\n|\n\
        combine |left I32, right I32| -> I32:\n    return left + right\n;\n\
        update |this ~Point, amount I32|:\n    ~this.x = amount\n;\n\
        point ~= Point()\n\
        ~point.update(amount = combine(left = 1, right = 2))\n\
        ~point.x = 3\n";
    let (ast, _, _) = parse_single_file_ast(source);
    let body = start_function_body(&ast);
    assert!(matches!(body[1].kind, NodeKind::ExpressionStatement(_)));
    assert!(matches!(body[2].kind, NodeKind::Assignment { .. }));
}

// --------------------------
//  Basic declarations
// --------------------------

#[test]
fn parses_mutable_and_explicitly_typed_declarations() {
    let (ast, _, _) = parse_single_file_ast("count ~= 1\nname String = \"Ada\"\n");

    let body = start_function_body(&ast);

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
    let (ast, _, _) =
        parse_single_file_ast("Point = |\n    x I32,\n|\n\norigin Point = Point(0)\n");

    let body = start_function_body(&ast);

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
    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("reserved-name rejection should produce a diagnosed result");
    };
    let (report, _) = reports
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
    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("reserved-name rejection should produce a diagnosed result");
    };
    let (report, _) = reports
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
    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("reserved-name rejection should produce a diagnosed result");
    };
    let (report, _) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("the failure contains its first error report");
    assert_eq!(report.context().strings().resolve(facts.name), "_true");
}

#[test]
fn redeclaration_retains_exact_duplicate_name_span() {
    let source = "value = \"π\"\nvalue I32 = 2\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0002", None);

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

#[test]
fn body_local_redeclaration_shapes_use_duplicate_declaration() {
    // Every unambiguous body-local declaration shape after a visible name is a redeclaration,
    // even when the annotation is not a builtin type token.
    for (source, name) in [
        (
            "holder || -> I32:\n    duplicated = 1\n    duplicated I32 = 2\n    return duplicated\n;\n",
            "duplicated",
        ),
        (
            "holder || -> I32:\n    duplicated ~= 0\n    duplicated ~= 1\n    return duplicated\n;\n",
            "duplicated",
        ),
        (
            "Record = |\n    field I32,\n|\n\nholder || -> I32:\n    duplicated = Record(field = 1)\n    duplicated Record = Record(field = 2)\n    return 1\n;\n",
            "duplicated",
        ),
        (
            "AccountId as I32\n\nholder || -> I32:\n    duplicated = 1\n    duplicated AccountId = 2\n    return duplicated\n;\n",
            "duplicated",
        ),
        (
            "Box type T = |\n    value T,\n|\n\nholder || -> I32:\n    duplicated = Box(value = 1)\n    duplicated Box of I32 = Box(value = 2)\n    return 1\n;\n",
            "duplicated",
        ),
        (
            "@core/io\n\nholder || -> I32:\n    duplicated = 1\n    duplicated io.input.Input = 2\n    return duplicated\n;\n",
            "duplicated",
        ),
        (
            "holder || -> I32:\n    duplicated = {1, 2}\n    duplicated {I32} = {3}\n    return 1\n;\n",
            "duplicated",
        ),
        (
            "holder || -> I32:\n    duplicated = 1\n    duplicated #= 2\n    return duplicated\n;\n",
            "duplicated",
        ),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0002", None);
        assert!(
            diagnostic
                .family_view::<DuplicateDeclarationView>()
                .is_some(),
            "the diagnostic is a duplicate declaration: {source}",
        );

        let mut builder = ExtendedSpanBuilder::new();
        // The earlier binding is the first authored occurrence and the redeclaration is the
        // second; a later use of the same name must not be mistaken for either.
        let mut occurrences = source.match_indices(name).map(|(start, _)| start as u32);
        let previous_start = occurrences.next().expect("the earlier name must occur");
        let duplicate_start = occurrences
            .next()
            .expect("the redeclaration name must occur");
        let duplicate_span = LocalSpan::exact(duplicate_start, name.len() as u32, &mut builder)
            .expect("the duplicate name span must fit");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, duplicate_span)),
            "{source}",
        );

        let previous_span = LocalSpan::exact(previous_start, name.len() as u32, &mut builder)
            .expect("the earlier name span must fit");
        assert_eq!(
            ast_error_label_spans(diagnostic),
            vec![Some(SourceSpan::new(
                SourceId::COMPILATION_ROOT,
                previous_span
            ))],
            "{source}",
        );
    }
}

#[test]
fn malformed_marker_spacing_on_an_existing_binding_keeps_the_marker_error() {
    // A binding marker after a visible name proves a declaration attempt, so its own spacing
    // diagnostic wins over expression parsing and matches the new-name behaviour.
    for (source, reason) in [
        (
            "holder || -> I32:\n    count ~= 0\n    count ~ = 1\n    return count\n;\n",
            "common_syntax_mistake.invalid_mutable_binding_spacing",
        ),
        (
            "holder || -> I32:\n    count ~= 0\n    count ~ I32 = 1\n    return count\n;\n",
            "common_syntax_mistake.invalid_mutable_binding_spacing",
        ),
        (
            "holder || -> I32:\n    count = 1\n    count # I32 = 2\n    return count\n;\n",
            "common_syntax_mistake.invalid_compile_time_binding_spacing",
        ),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0031", Some(reason));
    }
}

#[test]
fn body_redeclaration_of_a_visible_source_declaration_labels_it() {
    // A shared source declaration keeps its authored binding span, so the earlier top-level
    // binding is the secondary label instead of an unlabelled duplicate.
    for (source, name) in [
        (
            "value #= 1\n\nholder || -> I32:\n    value I32 = 2\n    return value\n;\n",
            "value",
        ),
        (
            "helper || -> I32:\n    return 1\n;\n\nholder || -> I32:\n    helper ~= 2\n    return 1\n;\n",
            "helper",
        ),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0002", None);
        let mut builder = ExtendedSpanBuilder::new();
        let mut occurrences = source.match_indices(name).map(|(start, _)| start as u32);
        let previous_start = occurrences.next().expect("the top-level name must occur");
        let duplicate_start = occurrences
            .next()
            .expect("the body redeclaration must occur");
        let duplicate_span = LocalSpan::exact(duplicate_start, name.len() as u32, &mut builder)
            .expect("the duplicate name span must fit");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, duplicate_span)),
            "{source}",
        );
        let previous_span = LocalSpan::exact(previous_start, name.len() as u32, &mut builder)
            .expect("the top-level name span must fit");
        assert_eq!(
            ast_error_label_spans(diagnostic),
            vec![Some(SourceSpan::new(
                SourceId::COMPILATION_ROOT,
                previous_span
            ))],
            "{source}: the shared declaration is the previous declaration",
        );
    }
}

#[test]
fn prefixed_write_updates_an_existing_mutable_binding() {
    parse_single_file_ast(
        "holder || -> I32:\n    count ~= 0\n    ~count = 1\n    return count\n;\n",
    );
}

#[test]
fn plain_write_to_an_existing_mutable_binding_requires_the_prefix() {
    let failure = parse_single_file_ast_diagnostic(
        "holder || -> I32:\n    count ~= 0\n    count = 1\n    return count\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.missing_write_prefix"),
    );
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
    // Prelude symbols are compiler-owned aliases without an authored binding location; an
    // explicit dependency alias keeps the authored alias span for the secondary label.
    for (source, name, previous_label) in [
        ("io = 1\n", "io", false),
        ("io I32 = 1\n", "io", false),
        ("prelude_print = 1\n", "prelude_print", false),
        ("prelude_print I32 = 1\n", "prelude_print", false),
        ("if true:\n    io = 1\n;\n", "io", false),
        (
            "pair || -> I32, I32:\n    return 1, 2\n;\nvalue, io = pair()\n",
            "io",
            false,
        ),
        (
            "pair || -> I32, I32:\n    return 1, 2\n;\nvalue I32, io I32 = pair()\n",
            "io",
            false,
        ),
        (
            "@core/io line as host_line\nhost_line I32 = 1\n",
            "host_line",
            true,
        ),
        ("@core/io as output\noutput = 1\n", "output", false),
        (
            "@core/io line as host_line\npair || -> I32, I32:\n    return 1, 2\n;\nvalue, host_line I32 = pair()\n",
            "host_line",
            true,
        ),
    ] {
        let failure =
            parse_single_file_ast_build_result_with_registry(source, body_prelude_registry())
                .err()
                .expect("a body binding must not replace a visible prelude binding");
        let failure = freeze_ast_diagnostic_failure(failure);
        let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
            panic!("the duplicate declaration should produce a diagnosed result");
        };
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0002", None);
        let facts = diagnostic
            .family_view::<DuplicateDeclarationView>()
            .expect("the diagnostic is a duplicate declaration");
        let (report, _) = reports
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
        let labels = ast_error_label_spans(diagnostic);
        if previous_label {
            let alias_start = source
                .find(name)
                .expect("the dependency alias name must be present")
                as u32;
            let alias_span =
                LocalSpan::exact(alias_start, name.len() as u32, &mut expected_builder)
                    .expect("the dependency alias span must fit the inline representation");
            assert_eq!(
                labels,
                vec![Some(SourceSpan::new(
                    SourceId::COMPILATION_ROOT,
                    alias_span
                ))],
                "{source}: the authored dependency alias is the earlier binding",
            );
        } else {
            assert!(
                labels.is_empty(),
                "{source}: prelude collisions have no secondary declaration label",
            );
        }
        assert_eq!(
            reports.iter().count(),
            1,
            "{source}: expected one duplicate diagnostic for {name}",
        );
    }
}

#[test]
fn body_local_prelude_lookup_preserves_namespace_and_function_calls() {
    let (result, _, _) = parse_single_file_ast_build_result_with_registry(
        "message = \"hello\"\nio.print(message)\nprelude_print(message)\n",
        body_prelude_registry(),
    )
    .expect("ordinary locals and original prelude calls must remain valid");
    let body = start_function_body(&result.ast);
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
fn multi_bind_reports_double_equals_as_equality_mistake() {
    let source = "a ~= 1\nb ~= 2\n~a, ~b == {3, 4}\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.equality_operator"),
    );
    let operator_start = source.rfind("==").expect("equality should be present") as u32;
    let operator_span = LocalSpan::exact(operator_start, 2, &mut ExtendedSpanBuilder::new())
        .expect("the operator span should fit inline");

    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator_span))
    );
}

#[test]
fn multi_bind_reports_reserved_keyword_target_name() {
    let source = "pair || -> I32, I32:\n    return 1, 2\n;\nbx, by = pair()\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the multi-bind target must report a reserved name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::Keyword);

    let keyword_start = source.rfind("by").expect("the reserved target must occur") as u32;
    let mut builder = ExtendedSpanBuilder::new();
    let keyword_span = LocalSpan::exact(keyword_start, "by".len() as u32, &mut builder)
        .expect("the keyword span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, keyword_span)),
    );
}

#[test]
fn reserved_binding_declaration_precedes_later_assignment_name_before_minus() {
    let source = "f |a I32| -> I32:\n    by = a\n    x = by - 1\n    return x\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the declaration must report a reserved name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::Keyword);

    let declaration_start = source
        .find("by")
        .expect("the declaration target must occur") as u32;
    let mut builder = ExtendedSpanBuilder::new();
    let declaration_span = LocalSpan::exact(declaration_start, "by".len() as u32, &mut builder)
        .expect("the keyword span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            declaration_span
        )),
        "the earlier declaration must be the first diagnostic, not the later minus",
    );

    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("the reserved declaration must produce a diagnosed result");
    };
    assert_eq!(
        reports.iter_severity(DiagnosticSeverity::Error).count(),
        1,
        "the reserved declaration must remain the only reported error",
    );
}

#[test]
fn assignment_name_before_spaced_minus_reports_reserved_collision() {
    let source = "f |a I32| -> I32:\n    x = by - 1\n    return x\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the assignment name must report a reserved name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::Keyword);

    let keyword_start = source.find("by").expect("the reserved name must occur") as u32;
    let mut builder = ExtendedSpanBuilder::new();
    let keyword_span = LocalSpan::exact(keyword_start, "by".len() as u32, &mut builder)
        .expect("the keyword span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, keyword_span)),
        "the collision must point at the reserved name, not the minus",
    );

    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("the assignment name must produce a diagnosed result");
    };
    assert_eq!(
        reports.iter_severity(DiagnosticSeverity::Error).count(),
        1,
        "the reserved name must remain the only reported error",
    );
}

#[test]
fn compound_assignment_name_before_spaced_minus_reports_reserved_collision() {
    let source = "f |a I32| -> I32:\n    x ~= 0\n    ~x += by - 1\n    return x\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the compound assignment name must report a reserved name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::Keyword);

    let keyword_start = source.find("by").expect("the reserved name must occur") as u32;
    let mut builder = ExtendedSpanBuilder::new();
    let keyword_span = LocalSpan::exact(keyword_start, "by".len() as u32, &mut builder)
        .expect("the keyword span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, keyword_span)),
        "the collision must point at the reserved name after the compound assignment",
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
    let (ast, _, string_table) = parse_single_file_ast("items {2} = {1, 2}\n");
    let body = start_function_body(&ast);

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
    let (ast, _, string_table) = parse_single_file_ast(
        r#"
    Names as {2 String}
    names Names = {"Priya"}
    "#,
    );
    let body = start_function_body(&ast);

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
    let (ast, _, string_table) = parse_single_file_ast("grid {2 {3 I32}} = {{1}, {2}}\n");
    let body = start_function_body(&ast);

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
    let (ast, _, string_table) = parse_single_file_ast(
        r#"
    make || -> {2 I32}:
        return {1}
    ;
    
    items {2 I32} = make()
    "#,
    );
    let body = start_function_body(&ast);

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
    let (ast, _, string_table) = parse_single_file_ast(
        r#"
    identity type Item |value Item| -> Item:
        return value
    ;
    
    items {2 I32} = {1}
    same = identity(items)
    "#,
    );
    let body = start_function_body(&ast);

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
    let (ast, _, string_table) = parse_single_file_ast("items ~{2 I32} = {}\n");
    let body = start_function_body(&ast);

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
    let (ast, _, string_table) = parse_single_file_ast("items {I32} = {1, 2}\n");
    let body = start_function_body(&ast);
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
                );
            let declaration = parse_declaration_syntax(
                &mut declaration_cursor,
                name,
                &mut strings,
                &mut builder,
                None,
            )
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
        let mut source_builder = SourceDatabaseBuilder::new(sources);
        source_builder.retain_span_builder(file_id, builder);
        source_builder
            .finish_source(file_id)
            .expect("the original table must freeze after the final span producer");
        source_builder
            .assemble_finalized_tables()
            .expect("the frozen declaration table should assemble");
        let sources = source_builder
            .finish()
            .expect("the original source owner should satisfy lifecycle invariants");
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
        let source = format!("{producer}\nvalue ~= 0\n~value = produce()\n");
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

#[test]
fn rejects_double_equals_statement_as_equality_mistake() {
    let failure = parse_single_file_ast_diagnostic("x = 1\nx == 1\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.equality_operator"),
    );
}

#[test]
fn typed_declaration_reports_equality_written_after_its_separator() {
    let source = "x I32 == 1\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.equality_operator"),
    );
    let operator_start = source.find("==").expect("operator should be present") as u32;
    let operator_span = LocalSpan::exact(operator_start, 2, &mut ExtendedSpanBuilder::new())
        .expect("the operator span should fit inline");

    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator_span))
    );
}

#[test]
fn rejects_discarded_comparison_statement_with_comparison_reason() {
    let failure = parse_single_file_ast_diagnostic("x = 1\nx is 1\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0025",
        Some("invalid_standalone_statement.comparison"),
    );
}

#[test]
fn doc_attaches_to_an_entry_local_declaration() {
    let (ast, path_fork, string_table) = parse_single_file_ast("$doc [:Local docs.]\nvalue = 1\n");
    assert_eq!(
        ast.doc_fragments.len(),
        1,
        "the declaration should retain one attached doc record"
    );
    let target = ast.doc_fragments[0].target;
    let target_name = path_fork
        .try_component(target)
        .expect("the doc target is a declaration path");
    assert_eq!(string_table.resolve(target_name), "value");
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Local docs."
    );
    assert!(
        ast.doc_fragments[0].span.is_some(),
        "source docs retain their authored operand span"
    );
}

#[test]
fn todo_attaches_to_a_return_statement_without_runtime_work() {
    let (ast, _path_fork, _string_table) = parse_single_file_ast(
        "calculate || -> I32:\n    $todo [: Implement the rest.]\n    return 1\n;\n",
    );
    assert!(
        ast.doc_fragments.is_empty(),
        "`$todo` is validation-only and publishes no metadata"
    );
}

#[test]
fn todo_before_a_top_level_runtime_fragment_is_accepted() {
    let (ast, _path_fork, _string_table) =
        parse_single_file_ast("$todo [: Runtime review.]\n~[: [1]]\n");
    assert!(
        ast.doc_fragments.is_empty(),
        "`$todo` never becomes documentation metadata"
    );
}

#[test]
fn doc_before_a_top_level_runtime_fragment_is_wrong_target() {
    let source = "$doc [: Runtime docs.]\n~[: [1]]\n";
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.wrong_target"),
    );
}

#[test]
fn adjacent_source_directive_operands_are_rejected() {
    let source = "calculate || -> I32:\n    $todo [: one] [: two]\n    return 1\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.unexpected_arguments"),
    );
}

#[test]
fn runtime_marker_rejects_a_note_template() {
    let failure = parse_single_file_ast_diagnostic("~[$note: hidden]\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.wrong_context"),
    );
}

#[test]
fn runtime_marker_rejects_a_multiline_note_at_the_authored_directive() {
    let source = "~[\n    $note: hidden\n]\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.wrong_context"),
    );
    let mut spans = ExtendedSpanBuilder::new();
    let note = LocalSpan::exact(
        source
            .find("$note")
            .expect("fixture contains the directive") as u32,
        "$note".len() as u32,
        &mut spans,
    )
    .expect("directive span fits");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, note)),
    );
}

#[test]
fn direct_note_produces_no_fragment() {
    let (ast, _path_fork, _string_table) = parse_single_file_ast("[$note: hidden]\n");
    assert!(
        ast.const_top_level_fragments.is_empty(),
        "a direct note must not become an empty const body fragment"
    );
}

#[test]
fn template_inside_an_executable_block_is_not_direct_metadata() {
    let source = "if false:\n[:unexpected]\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0025",
        Some("invalid_standalone_statement.standalone_template"),
    );
}

#[test]
fn removed_hash_template_spelling_is_rejected() {
    let source = "#[: removed]\n";
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0002", None);
}

#[test]
fn doc_attaches_to_a_function_local_declaration() {
    let source =
        "calculate || -> I32:\n    $doc [:Local docs.]\n    value = 1\n    return value\n;\n";
    let (ast, _path_fork, string_table) = parse_single_file_ast(source);
    assert_eq!(ast.doc_fragments.len(), 1);
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Local docs."
    );
}

#[test]
fn doc_operand_accepts_a_compile_time_string_binding() {
    let source = "label #= \"Constant docs\"\n$doc label\ncalculate || -> I32:\n    return 1\n;\n";
    let (ast, _path_fork, string_table) = parse_single_file_ast(source);
    assert_eq!(ast.doc_fragments.len(), 1);
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Constant docs"
    );
}

#[test]
fn template_head_doc_and_todo_forms_are_rejected() {
    for source in ["[$doc: text]\n", "[$todo: text]\n"] {
        let failure = parse_single_file_ast_diagnostic(source);
        assert_ast_diagnostic(
            &failure,
            "MOTH-SYNTAX-0021",
            Some("invalid_template_directive.wrong_context"),
        );
    }
}

#[test]
fn local_source_directive_target_must_start_on_a_later_line() {
    let failure = parse_single_file_ast_diagnostic("f ||:\n    $doc [:x] value = 1\n;\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.dangling_modifier"),
    );
}

#[test]
fn local_multiline_directive_run_attaches_each_annotation() {
    let (ast, _path_fork, string_table) =
        parse_single_file_ast("f ||:\n    $doc [:First.]\n    $todo [:Later.]\n    value = 1\n;\n");
    assert_eq!(ast.doc_fragments.len(), 1);
    assert_eq!(string_table.resolve(ast.doc_fragments[0].value), "First.");
}

#[test]
fn local_multiline_duplicate_directives_are_rejected() {
    let failure = parse_single_file_ast_diagnostic(
        "f ||:\n    $todo [:First.]\n    $todo [:Second.]\n    value = 1\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.duplicate_modifier"),
    );
}

#[test]
fn local_doc_attaches_to_explicitly_typed_declaration() {
    let (ast, _path_fork, string_table) =
        parse_single_file_ast("f ||:\n    $doc [:Typed docs.]\n    text #String = \"x\"\n;\n");
    assert_eq!(ast.doc_fragments.len(), 1);
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Typed docs."
    );
}

#[test]
fn generic_body_local_docs_publish_one_authored_fragment() {
    let (ast, _path_fork, string_table) = parse_single_file_ast(
        "draw type T |name T| -> String:\n    $doc [:Local docs.]\n    text #String = \"x\"\n    return text\n;\nfirst = draw(1)\nsecond = draw(\"a\")\n",
    );
    assert_eq!(
        ast.doc_fragments.len(),
        1,
        "a generic body must publish its authored local docs once, not once per instance"
    );
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Local docs."
    );
}

#[test]
fn same_line_runtime_fragments_fall_back_to_ordinary_grammar() {
    let failure = parse_single_file_ast_diagnostic("~[:one] ~[:two]\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.mutable_marker_on_non_receiver_call"),
    );
}

#[test]
fn local_doc_attaches_to_a_nominally_typed_declaration() {
    let source = "Note = |text String|\nf || -> String:\n    $doc [:Nominal docs.]\n    note Note = Note(text = \"x\")\n    return note.text\n;\n";
    let (ast, _path_fork, string_table) = parse_single_file_ast(source);
    assert_eq!(ast.doc_fragments.len(), 1);
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Nominal docs."
    );
}

#[test]
fn source_directive_at_a_match_arm_boundary_is_dangling() {
    let source = "Signal ::\n    Ready,\n    Waiting,\n;\nf || -> String:\n    value = Signal::Ready\n    if value is:\n        Ready =>\n            $todo [:dangling]\n        Waiting => return \"waiting\"\n    ;\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.dangling_modifier"),
    );
}

#[test]
fn nested_entry_block_doc_attaches_to_the_local_declaration() {
    let source = "if false:\n    $doc [:Block docs.]\n    value #String = \"x\"\n;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    assert_eq!(ast.doc_fragments.len(), 1);
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "Block docs."
    );
    let target_name = path_fork
        .try_component(ast.doc_fragments[0].target)
        .expect("the nested local declaration owns its doc target");
    assert_eq!(string_table.resolve(target_name), "value");
}

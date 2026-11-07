//! Standalone `-> !` call placement tests.
//!
//! WHAT: checks where a call whose published contract is Never is accepted as a statement and
//!       where it is rejected as a value or with a postfix continuation.
//! WHY: `NeverCall` has no value by contract, so its acceptance and rejection must be proven
//!       through the same authored source path users write, not only through constructed nodes.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::compiler_errors::CompilationResult;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionDef, ExternalFunctionLowerings,
    ExternalPackageRegistry, ExternalParameter, ExternalSignatureType, external_never,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::{
    ast_error_primary_span, first_ast_error_view, freeze_ast_diagnostic_failure,
    parse_single_file_ast, parse_single_file_ast_build_result,
    parse_single_file_ast_build_result_with_registry, parse_single_file_ast_diagnostic,
    parse_single_file_ast_result,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

const VALUE_USE_CODE: &str = "MOTH-RULE-0054";
const VALUE_USE_REASON: &str = "invalid_call_shape.never_call_used_as_value";
const HANDLED_REASON: &str = "invalid_call_shape.never_call_handled";

/// A valid `-> !` declaration every placement case can call.
const FATAL: &str = "fatal || -> !:\n    assert(false, \"fatal\")\n;\n\n";

fn with_fatal(body: &str) -> String {
    format!("{FATAL}{body}")
}

fn assert_value_use(source: &str, context: &str) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = first_ast_error_view(&failure);
    assert_eq!(diagnostic.descriptor().code, VALUE_USE_CODE, "{context}");
    assert_eq!(diagnostic.reason_key(), Some(VALUE_USE_REASON), "{context}");
}

fn assert_handled(source: &str, context: &str) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = first_ast_error_view(&failure);
    assert_eq!(diagnostic.descriptor().code, VALUE_USE_CODE, "{context}");
    assert_eq!(diagnostic.reason_key(), Some(HANDLED_REASON), "{context}");
}

const UNHANDLED_ERROR_CODE: &str = "MOTH-RULE-0051";
const UNHANDLED_ERROR_REASON: &str = "invalid_fallible_handling.unhandled_error_return";

/// A fallible producer every Never-operand case can pass as an argument or receiver.
const CAN_ERROR: &str = "can_error || -> I32, Error!:\n    return 1\n;\n\n";

fn assert_unhandled_error(source: &str, context: &str) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = first_ast_error_view(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        UNHANDLED_ERROR_CODE,
        "{context}"
    );
    assert_eq!(
        diagnostic.reason_key(),
        Some(UNHANDLED_ERROR_REASON),
        "{context}"
    );
}

/// Asserts the unhandled typed-error diagnostic points at the fallible argument itself.
///
/// WHAT: checks the primary location is inside the authored `can_error()` occurrence rather
///       than the Never call or its callee.
/// WHY: a deferred Never call completes outside its authored call site, so a diagnostic that
///      named the call span would hide which argument actually produced the unhandled error.
fn assert_unhandled_error_points_at_argument(
    failure: &CompilationResult<()>,
    source: &str,
    context: &str,
) {
    const ARGUMENT: &str = "can_error()";
    let diagnostic = first_ast_error_view(failure);
    assert_eq!(
        diagnostic.descriptor().code,
        UNHANDLED_ERROR_CODE,
        "{context}"
    );
    assert_eq!(
        diagnostic.reason_key(),
        Some(UNHANDLED_ERROR_REASON),
        "{context}"
    );
    let span = ast_error_primary_span(diagnostic)
        .unwrap_or_else(|| panic!("{context} must carry a primary source location"));
    assert_eq!(span.source(), SourceId::COMPILATION_ROOT, "{context}");
    let argument_start = source
        .find(ARGUMENT)
        .unwrap_or_else(|| panic!("{context} source must contain {ARGUMENT}"))
        as u32;
    let argument_end = argument_start + ARGUMENT.len() as u32;
    let range = span
        .local()
        .inline_range()
        .unwrap_or_else(|| panic!("{context} argument span must fit inline"));
    assert!(
        range.start() >= argument_start && range.end() <= argument_end,
        "{context} primary location must name the fallible argument, not the Never call: {range:?}"
    );
}

#[test]
fn never_call_rejects_unhandled_fallible_argument() {
    let source = format!(
        "{CAN_ERROR}fatal |code I32| -> !:\n    assert(false, \"fatal\")\n;\n\nrun ||:\n    fatal(can_error())\n;\n"
    );
    assert_unhandled_error(&source, "a free Never call argument");
}

#[test]
fn receiver_method_never_call_rejects_unhandled_fallible_argument() {
    let source = "Box = | value I32 |\n\
         can_error || -> I32, Error!:\n    return 1\n;\n\n\
         stop |this Box, code I32| -> !:\n    assert(false, \"stop\")\n;\n\n\
         run ||:\n    box = Box(value = 1)\n    box.stop(can_error())\n;\n";
    assert_unhandled_error(source, "a receiver-method Never call argument");
}

#[test]
fn never_call_rejects_unhandled_fallible_receiver() {
    let source = "Box = | value I32 |\n\
         can_error || -> Box, Error!:\n    return Box(value = 1)\n;\n\n\
         stop |this Box| -> !:\n    assert(false, \"stop\")\n;\n\n\
         run ||:\n    can_error().stop()\n;\n";
    assert_unhandled_error(source, "a Never call receiver");
}

#[test]
fn never_call_rejects_unhandled_fallible_argument_in_error_returning_caller() {
    let source = format!(
        "{CAN_ERROR}fatal |code I32| -> !:\n    assert(false, \"fatal\")\n;\n\nrun || -> I32, Error!:\n    fatal(can_error())\n;\n"
    );
    assert_unhandled_error(
        &source,
        "a Never call argument inside an error-returning caller",
    );
}

#[test]
fn retained_never_call_rejects_unhandled_fallible_argument() {
    let source = format!(
        "{CAN_ERROR}fatal |code I32, other I32| -> !:\n    assert(false, \"fatal\")\n;\n\nrun ||:\n    count = 1\n    fatal(count, can_error())\n;\n"
    );
    assert_unhandled_error(&source, "a retained Never call argument");
}

#[test]
fn deferred_generic_never_call_rejects_unhandled_fallible_argument() {
    let source = format!(
        "{CAN_ERROR}abort type A |value A, other I32| -> !:\n    assert(false, \"abort\")\n;\n\nrun || -> I32, Error!:\n    count = 1\n    abort(count, can_error())\n;\n"
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    assert_unhandled_error_points_at_argument(
        &failure,
        &source,
        "a deferred generic Never call argument in an error-returning caller",
    );
}

#[test]
fn retained_never_call_cannot_be_a_multi_bind_value() {
    let source = "fatal |code I32| -> !:\n    assert(false, \"fatal\")\n;\n\n\
                  stop ||:\n    count = 1\n    a, b = fatal(count)\n;\n";
    assert_value_use(
        source,
        "a retained Never call as a multi-bind right-hand side",
    );
}

#[test]
fn retained_receiver_never_call_cannot_be_a_multi_bind_value() {
    let source = "Box = | value I32 |\n\
         stop |this Box, code I32| -> !:\n    assert(false, \"stop\")\n;\n\n\
         run ||:\n    box = Box(value = 1)\n    count = 1\n    a, b = box.stop(count)\n;\n";
    assert_value_use(
        source,
        "a retained receiver Never call as a multi-bind right-hand side",
    );
}

fn assert_single_never_statement(source: &str, function: &str, context: &str) {
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, function);
    assert!(
        matches!(
            body,
            [AstNode {
                kind: NodeKind::NeverCall(_),
                ..
            }]
        ),
        "{context} must lift to a terminating statement"
    );
}

/// Builds the binding-backed external Never functions the external placement cases import.
///
/// `die` takes no parameters for the immediate-call cases; `halt` takes two, so a retained
/// numeric argument plus a fallible argument can drive the deferred external completion path.
fn external_never_registry() -> ExternalPackageRegistry {
    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_function(ExternalFunctionDef {
            name: "die".to_owned(),
            parameters: Vec::new(),
            return_contract: external_never(),
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("external Never signature must register");
    registry
        .register_function(ExternalFunctionDef {
            name: "halt".to_owned(),
            parameters: vec![
                ExternalParameter {
                    language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(
                        FixedScalar::I32,
                    )),
                    access_kind: ExternalAccessKind::Shared,
                },
                ExternalParameter {
                    language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(
                        FixedScalar::I32,
                    )),
                    access_kind: ExternalAccessKind::Shared,
                },
            ],
            return_contract: external_never(),
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("parameterised external Never signature must register");
    registry
}

fn parse_external_never_ast(source: &str) -> (Ast, PathInternerFork, StringTable) {
    let (build_result, path_fork, string_table) =
        parse_single_file_ast_build_result_with_registry(source, external_never_registry())
            .expect("external Never call source should build");
    (build_result.ast, path_fork, string_table)
}

fn parse_external_never_ast_diagnostic(source: &str) -> CompilationResult<()> {
    match parse_single_file_ast_build_result_with_registry(source, external_never_registry()) {
        Ok(_) => panic!("external Never call value source should fail during frontend parsing"),
        Err(failure) => freeze_ast_diagnostic_failure(failure),
    }
}

#[test]
fn free_never_call_is_accepted_as_a_standalone_statement() {
    assert_single_never_statement(
        &with_fatal("stop || -> !:\n    fatal()\n;\n"),
        "stop",
        "a free Never call",
    );
}

#[test]
fn receiver_method_never_call_is_accepted_as_a_standalone_statement() {
    let source = "Box = | value I32 |\n\
                  abort |this Box| -> !:\n    assert(false, \"abort\")\n;\n\n\
                  stop |box Box| -> !:\n    box.abort()\n;\n";
    assert_single_never_statement(source, "stop", "a receiver method Never call");
}

#[test]
fn generic_never_call_is_accepted_as_a_standalone_statement() {
    let source = "abort type A |value A| -> !:\n    assert(false, \"abort\")\n;\n\n\
                  stop |value I32| -> !:\n    abort(value)\n;\n";
    assert_single_never_statement(source, "stop", "a generic Never call");
}

#[test]
fn external_never_call_is_accepted_as_a_standalone_statement() {
    let source = "@test/default die\nstop || -> !:\n    die()\n;\n";
    let (ast, paths, strings) = parse_external_never_ast(source);
    let body = function_body_by_name(&ast, &paths, &strings, "stop");
    assert!(
        matches!(
            body,
            [AstNode {
                kind: NodeKind::NeverCall(_),
                ..
            }]
        ),
        "an imported external Never call must lift to a terminating statement"
    );
}

#[test]
fn external_never_call_cannot_be_a_value() {
    let source = "@test/default die\nvalue || -> I32:\n    return die()\n;\n";
    let failure = parse_external_never_ast_diagnostic(source);
    let diagnostic = first_ast_error_view(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        VALUE_USE_CODE,
        "an imported external Never call in value position"
    );
    assert_eq!(diagnostic.reason_key(), Some(VALUE_USE_REASON));
}

#[test]
fn deferred_external_never_call_rejects_unhandled_fallible_argument() {
    let source = format!(
        "@test/default halt\n{CAN_ERROR}run || -> I32, Error!:\n    count = 1\n    halt(count, can_error())\n;\n"
    );
    let failure = parse_external_never_ast_diagnostic(&source);
    assert_unhandled_error_points_at_argument(
        &failure,
        &source,
        "a deferred external Never call argument in an error-returning caller",
    );
}

#[test]
fn generic_never_statement_records_a_concrete_instantiation_request() {
    let source = "abort type A |value A| -> !:\n    assert(false, \"abort\")\n;\n\n\
                  stop |value I32| -> !:\n    abort(value)\n;\n";
    let (build_result, path_fork, string_table) =
        parse_single_file_ast_build_result(source).expect("generic Never statement should build");

    let [request] = build_result.deferred_generic_requests.as_slice() else {
        panic!("one generic Never statement should emit one concrete request");
    };
    assert_eq!(
        path_fork
            .component(request.key.function_path)
            .map(|component| string_table.resolve(component)),
        Some("abort")
    );
    assert_eq!(
        request.key.type_arguments.as_ref(),
        &[builtin_type_ids::fixed_scalar(FixedScalar::I32)],
        "the statement's argument must produce a concrete I32 request"
    );
}

#[test]
fn generic_never_statement_in_inactive_static_branch_is_discarded() {
    let source = "disabled #= false\n\
                  abort type A |value A| -> !:\n    assert(false, \"abort\")\n;\n\n\
                  stop |value I32| -> !:\n    if disabled:\n        abort(value)\n    ;\n    assert(false, \"stop\")\n;\n";
    let (build_result, _, _) = parse_single_file_ast_build_result(source)
        .expect("an inactive generic Never call must not fail the active body");

    assert!(
        build_result.deferred_generic_requests.is_empty(),
        "an inactive static branch must discard its concrete generic request"
    );
}

#[test]
fn retained_never_call_with_numeric_argument_completes_to_a_statement() {
    let source = "fatal |code I32| -> !:\n    assert(false, \"fatal\")\n;\n\n\
                  stop || -> !:\n    count = 1\n    fatal(count)\n;\n";
    let (ast, paths, strings) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &paths, &strings, "stop");
    assert!(matches!(
        body.last().map(|node| &node.kind),
        Some(NodeKind::NeverCall(_))
    ));
}

const RETAINED_METHOD: &str = "Box = | value I32 |\n\
    make |value I32| -> Box:\n    return Box(value = value)\n;\n\
    stop |this Box| -> !:\n    assert(false, \"stop\")\n;\n";

#[test]
fn retained_postfix_never_method_is_a_statement() {
    let source =
        format!("{RETAINED_METHOD}run || -> !:\n    count = 1\n    make(count).stop()\n;\n");
    let (ast, paths, strings) = parse_single_file_ast(&source);
    let body = function_body_by_name(&ast, &paths, &strings, "run");
    assert!(matches!(
        body.last().map(|node| &node.kind),
        Some(NodeKind::NeverCall(_))
    ));
}

#[test]
fn retained_postfix_never_method_cannot_be_a_value() {
    let source = format!(
        "{RETAINED_METHOD}run || -> I32:\n    count = 1\n    return make(count).stop()\n;\n"
    );
    assert_value_use(&source, "a retained postfix Never method");
}

#[test]
fn binary_operator_operand_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("compute || -> I32:\n    return fatal() + 1\n;\n"),
        "a binary operand",
    );
}

#[test]
fn call_argument_rejects_a_never_call() {
    assert_value_use(
        &with_fatal(
            "consume |value I32| -> I32:\n    return value\n;\n\
             compute || -> I32:\n    return consume(fatal())\n;\n",
        ),
        "a call argument",
    );
}

#[test]
fn receiver_method_argument_rejects_a_never_call() {
    let source = format!(
        "Box = | value I32 |\n\
         consume |this Box, value I32| -> I32:\n    return value\n;\n\
         \n{FATAL}compute || -> I32:\n    box = Box(value = 1)\n    return box.consume(fatal())\n;\n"
    );
    assert_value_use(&source, "a receiver method argument");
}

#[test]
fn constructor_field_rejects_a_never_call() {
    assert_value_use(
        &format!(
            "Box = | value I32 |\n\n{FATAL}make || -> Box:\n    return Box(value = fatal())\n;\n"
        ),
        "a constructor field",
    );
}

#[test]
fn cast_operand_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("narrow || -> U8:\n    return cast fatal()\n;\n"),
        "a cast operand",
    );
}

#[test]
fn template_interpolation_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("announce ||:\n    io.line([: value=[fatal()]])\n;\n"),
        "a template interpolation",
    );
}

#[test]
fn collection_element_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("collect || -> {I32}:\n    return {fatal()}\n;\n"),
        "a collection element",
    );
}

#[test]
fn map_element_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("collect || -> {I32 = I32}:\n    return {1 = fatal()}\n;\n"),
        "a map element",
    );
}

#[test]
fn condition_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("check ||:\n    if fatal():\n        io.line(\"unreachable\")\n    ;\n;\n"),
        "a condition",
    );
}

#[test]
fn declaration_rhs_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("value ||:\n    count I32 = fatal()\n;\n"),
        "a declaration right-hand side",
    );
}

#[test]
fn assignment_rhs_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("value ||:\n    count ~= 0\n    ~count = fatal()\n;\n"),
        "an assignment right-hand side",
    );
}

#[test]
fn assert_argument_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("check ||:\n    assert(true, fatal())\n;\n"),
        "an assertion argument",
    );
}

#[test]
fn assert_condition_rejects_a_never_call() {
    assert_value_use(
        &with_fatal("check ||:\n    assert(fatal(), \"message\")\n;\n"),
        "an assertion condition",
    );
}

#[test]
fn inline_value_branch_rejects_a_never_call() {
    assert_value_use(
        &with_fatal(
            "choose |ready Bool| -> String:\n    value = if ready then \"yes\" else fatal()\n    return value\n;\n",
        ),
        "an inline value-producing branch",
    );
}

#[test]
fn block_value_branch_accepts_a_never_call() {
    let source = format!(
        "{FATAL}choose |ready Bool| -> String:\n    value = if ready:\n        then \"yes\"\n    else\n        fatal()\n    ;\n    return value\n;\n"
    );
    let result = parse_single_file_ast_result(&source);
    assert!(
        result.is_ok(),
        "a block-form value producer may complete one branch by diverging"
    );
}

#[test]
fn postfix_error_propagation_is_rejected() {
    assert_handled(&with_fatal("check ||:\n    fatal()!\n;\n"), "postfix `!`");
}

#[test]
fn postfix_option_propagation_is_rejected() {
    assert_handled(&with_fatal("check ||:\n    fatal()?\n;\n"), "postfix `?`");
}

#[test]
fn catch_is_rejected() {
    assert_handled(
        &with_fatal("check ||:\n    fatal() catch:\n        io.line(\"fallback\")\n    ;\n;\n"),
        "`catch`",
    );
}

#[test]
fn member_access_is_rejected() {
    assert_handled(
        &with_fatal("check ||:\n    fatal().value\n;\n"),
        "member access",
    );
}

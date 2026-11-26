use super::{BorrowChecker, CallResultFacts, FieldKey};
use crate::compiler_frontend::analysis::borrow_checker::{OptionalTransferStatus, check_borrows};
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

fn with_results(source: &str, check: impl FnOnce(&dyn Fn(&str) -> FunctionId, &BorrowChecker<'_>)) {
    let (ast, mut paths, mut strings) = parse_single_file_ast(source);
    let packages = default_external_package_registry(&mut strings);
    let hir = lower_hir(ast, &mut strings, &mut paths, &packages);
    let mut checker = BorrowChecker::new(&hir, &hir.type_environment, &packages, &paths, &strings);
    checker.build_public_call_summaries().unwrap();
    let by_name = |name: &str| {
        hir.functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .and_then(|path| paths.component(path))
                    .is_some_and(|component| strings.resolve(component) == name)
            })
            .unwrap()
            .id
    };
    check(&by_name, &checker);
}

#[test]
fn typed_result_paths_prove_fresh_wrappers_and_authored_parameter_slots() {
    with_results(
        r#"
Link = |items {I32}|

wrap |items {I32}| -> Link:
    temp = Link(items)
    return temp
;

wrap_copy |items {I32}| -> Link:
    return Link(copy items)
;

unwrap_link |link Link| -> {I32}:
    return link.items
;

unwrap_temp |items {I32}| -> {I32}:
    return Link(items).items
;

ordered |first {I32}, second {I32}| -> I32, {I32}, {I32}:
    return 1, second, copy first
;

agree |flag Bool, items {I32}| -> Link:
    if flag:
        return Link(items)
    ;
    return Link(items)
;
"#,
        |function, checker| {
            let results = checker.build_typed_local_results().unwrap();
            let wrap = function("wrap");
            let result_type = checker
                .module
                .functions
                .iter()
                .find(|function| function.id == wrap)
                .unwrap()
                .return_contract
                .ordinary()
                .unwrap()
                .success()[0];
            let layout = checker
                .module
                .structs
                .iter()
                .find(|layout| layout.frontend_type_id == result_type)
                .unwrap();
            let items = FieldKey::Field(layout.fields[0].id.0);
            let parameter = |index| CallResultFacts::Parameter {
                index,
                path: Vec::new(),
            };
            let fresh = || CallResultFacts::Fresh {
                children: Vec::new(),
            };
            assert_eq!(
                results[&wrap],
                [Some(CallResultFacts::Fresh {
                    children: vec![(items, parameter(0))],
                })]
            );
            assert_eq!(
                results[&function("wrap_copy")],
                [Some(CallResultFacts::Fresh {
                    children: vec![(items, fresh())],
                })]
            );
            assert_eq!(
                results[&function("unwrap_link")],
                [Some(CallResultFacts::Parameter {
                    index: 0,
                    path: vec![items],
                })]
            );
            assert_eq!(
                results[&function("agree")],
                [Some(CallResultFacts::Fresh {
                    children: vec![(items, parameter(1))],
                })]
            );
            assert_eq!(results[&function("unwrap_temp")], [Some(parameter(0))]);
            assert_eq!(
                results[&function("ordered")],
                [None, Some(parameter(1)), Some(fresh())]
            );
        },
    );
}

#[test]
fn typed_result_paths_require_unchanged_proven_storage() {
    with_results(
        r#"
Link = |items {I32}|
Pair = |left {I32}, right {I32}|

changed_field |items {I32}| -> Link:
    value ~= Link(items)
    ~value.items = {7}
    return value
;

mutator |items ~{I32}|:
    ~items.push(7)
;

changed_call |items ~{I32}| -> Link:
    value = Link(items)
    mutator(~items)
    return value
;

unproven |items {I32}| -> Link:
    return wrap(items)
;

wrap |items {I32}| -> Link:
    return Link(items)
;

choose |flag Bool, first {I32}, second {I32}| -> Link:
    if flag:
        return Link(first)
    ;
    return Link(second)
;

shared_fresh_fields || -> Pair:
    items ~= {1}
    return Pair(items, items)
;

shared_fresh_slots || -> Link, {I32}:
    items ~= {1}
    return Link(items), items
;

copy_may_share_internally |pair Pair| -> Pair:
    return copy pair
;

recursive |items {I32}| -> Link:
    return recursive(items)
;
"#,
        |function, checker| {
            let results = checker.build_typed_local_results().unwrap();
            for name in [
                "changed_field",
                "changed_call",
                "unproven",
                "recursive",
                "choose",
                "shared_fresh_fields",
                "copy_may_share_internally",
            ] {
                assert_eq!(results[&function(name)], [None], "{name}");
            }
            assert_eq!(results[&function("shared_fresh_slots")], [None, None]);
        },
    );
}

#[test]
fn typed_result_paths_keep_returned_option_payload_borrowed() {
    with_results(
        r#"
Link = |items {I32}|

stop || -> !:
    assert(false, "missing link")
;

unwrap_link |maybe Link?| -> Link:
    if maybe is:
        |present| => return present
        none => stop()
    ;
;

probe || -> I32:
    source ~Link? = Link({1})
    view = unwrap_link(source)
    return view.items.length()
;

~[:[probe()]]
"#,
        |function, checker| {
            let unwrap_link = function("unwrap_link");
            assert_eq!(
                checker.public_call_summaries[&unwrap_link].return_alias,
                FunctionReturnAliasSummary::AliasParams(vec![0]),
            );
            assert_eq!(
                checker.build_typed_local_results().unwrap()[&unwrap_link],
                [Some(CallResultFacts::Parameter {
                    index: 0,
                    path: vec![FieldKey::Payload {
                        variant: 1,
                        field: 0,
                    }],
                })]
            );

            let argument = checker
                .module
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Call {
                        target: CallTarget::Local(id),
                        args,
                        ..
                    } if *id == unwrap_link => Some(checker.module.expressions.values(*args)[0]),
                    _ => None,
                })
                .expect("probe calls unwrap_link");
            let report = check_borrows(
                checker.module,
                checker.type_environment,
                checker.external_package_registry,
                checker.path_fork,
                checker.string_table,
            )
            .expect("the caller can read the returned payload after the call");
            assert_eq!(
                report
                    .analysis
                    .value_fact(argument)
                    .expect("the unwrap argument has a borrow fact")
                    .optional_transfer,
                OptionalTransferStatus::Borrow,
                "the final direct use cannot transfer an option whose payload is retained"
            );
        },
    );
}

#[test]
fn opaque_result_paths_keep_only_proven_coarse_provenance() {
    for (source, name, return_alias) in [
        (
            include_str!(
                "../../../../../tests/cases/runtime_call_opaque_option_retained_result_rejected/input/@entry.moth"
            ),
            "wrap_option",
            FunctionReturnAliasSummary::AliasParams(vec![0]),
        ),
        (
            include_str!(
                "../../../../../tests/cases/runtime_call_opaque_option_fresh_result/input/@entry.moth"
            ),
            "fresh_wrap_option",
            FunctionReturnAliasSummary::Fresh,
        ),
        (
            include_str!(
                "../../../../../tests/cases/runtime_call_opaque_projected_store_result_rejected/input/@entry.moth"
            ),
            "store_items",
            FunctionReturnAliasSummary::AliasParams(vec![0]),
        ),
        (
            r#"
Link = |items {I32}|

store_through_alias |items {I32}| -> Link:
    holder ~= Link({0})
    writer ~= holder
    ~writer.items = items
    return holder
;
"#,
            "store_through_alias",
            FunctionReturnAliasSummary::AliasParams(vec![0]),
        ),
        (
            r#"
Link = |items {I32}|
Outer = |link Link|

store_through_projected_writer |items {I32}| -> Outer:
    holder ~= Link({0})
    writer ~= holder.items
    ~writer = items
    return Outer(holder)
;
"#,
            "store_through_projected_writer",
            FunctionReturnAliasSummary::AliasParams(vec![0]),
        ),
        (
            r#"
Link = |items {I32}|

store_into_unreturned_scratch |items {I32}| -> Link:
    fresh ~= {0}
    scratch ~= Link(fresh)
    ~scratch.items = items
    return Link(fresh)
;
"#,
            "store_into_unreturned_scratch",
            FunctionReturnAliasSummary::Fresh,
        ),
        (
            r#"
Link = |items {I32}, tag I32|

tags |left Link, right Link| -> I32, I32:
    return left.tag, right.tag
;

store_after_scalar_results |items {I32}| -> Link:
    scratch ~= Link({0}, 1)
    returned ~= Link({1}, 2)
    left, right = tags(returned, scratch)
    ~scratch.items = items
    return returned
;
"#,
            "store_after_scalar_results",
            FunctionReturnAliasSummary::Fresh,
        ),
        (
            r#"
Link = |items {I32}|
Outer = |link Link|

store_through_retained_holder |items {I32}| -> Link:
    holder ~= Link({0})
    scratch ~= Outer(holder)
    ~scratch.link.items = items
    return holder
;
"#,
            "store_through_retained_holder",
            FunctionReturnAliasSummary::AliasParams(vec![0]),
        ),
    ] {
        with_results(source, |function, checker| {
            let id = function(name);
            assert_eq!(
                checker.build_typed_local_results().unwrap()[&id],
                [None],
                "{name}"
            );
            assert_eq!(
                checker.public_call_summaries[&id].return_alias, return_alias,
                "{name}"
            );
        });
    }
}

use super::{BorrowChecker, IndexCounts, build_sparse_index_for_test};
use crate::compiler_frontend::analysis::borrow_checker::OptionalTransferStatus;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationFacts, CallAllocationSource, CallField, CallLaneSummary, CallOutputPath,
    CallParameterPath, CallResultPath, CallSharing, CallSharingCondition,
    FunctionReturnAliasSummary,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker_with_types;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{LoweredHirFixture, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

struct ResultFixture {
    hir: LoweredHirFixture,
    packages: Arc<ExternalPackageRegistry>,
    paths: PathInternerFork,
    strings: StringTable,
}

impl ResultFixture {
    fn new(source: &str) -> Self {
        let (ast, mut paths, mut strings) = parse_single_file_ast(source);
        let packages = default_external_package_registry(&mut strings);
        let hir = lower_hir(ast, &mut strings, &mut paths, &packages);
        Self {
            hir,
            packages,
            paths,
            strings,
        }
    }

    fn checker(&self) -> BorrowChecker<'_> {
        BorrowChecker::new(
            &self.hir,
            &self.hir.type_environment,
            &self.packages,
            &self.paths,
            &self.strings,
        )
    }
}

fn named_function<'a>(checker: &BorrowChecker<'a>, name: &str) -> &'a HirFunction {
    checker
        .module
        .functions
        .iter()
        .find(|function| {
            checker.module.side_table.resolve_function_name(
                function.id,
                checker.path_fork,
                checker.string_table,
            ) == Some(name)
        })
        .unwrap_or_else(|| panic!("fixture should lower function {name}"))
}

#[test]
#[ignore = "reads locally generated sparse-index measurement inputs"]
fn measure_sparse_index_from_generated_hir() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/bc06-review/b2x/measure");
    for family in ["scalar", "independent", "correlated", "receivers"] {
        let mut previous = None::<IndexCounts>;
        for size in [16, 64, 256] {
            let name = format!("{family}-{size}");
            let path = root.join(&name).join("@entry.moth");
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

            // Parse source text with the harness's synthetic module identity.
            // The on-disk family names are never interpreted as Moth identifiers.
            let started = Instant::now();
            let (ast, mut paths, mut strings) = parse_single_file_ast(&source);
            let packages = default_external_package_registry(&mut strings);
            let parse_time = started.elapsed();
            let started = Instant::now();
            let hir = lower_hir(ast, &mut strings, &mut paths, &packages);
            let hir_time = started.elapsed();
            let started = Instant::now();
            let mut checker =
                BorrowChecker::new(&hir, &hir.type_environment, &packages, &paths, &strings);
            let indexes = build_sparse_index_for_test(&mut checker).unwrap();
            let builder_time = started.elapsed();

            let mut work = IndexCounts::default();
            let mut ports = 0;
            let mut guards = 0;
            let mut conditional_supports = 0;
            let mut conditional_arms = 0;
            for index in indexes.values() {
                work += index.counts;
                ports += index.ports.len();
                guards += index.guards.len();
                conditional_supports += index.conditional_supports.len();
                conditional_arms += index.conditional_arm_count;
            }
            let statements = hir
                .blocks
                .iter()
                .map(|block| block.statements.len())
                .sum::<usize>();
            eprintln!(
                "{name}: parse={parse_time:?} lower_hir={hir_time:?} sparse_builder={builder_time:?} hir_functions={} hir_blocks={} hir_statements={statements} ports={ports} guards={guards} conditional_supports={conditional_supports} conditional_arms={conditional_arms} work={work:?}",
                hir.functions.len(),
                hir.blocks.len()
            );
            assert!(work.blocks <= hir.blocks.len(), "{name}: {work:?}");
            assert!(work.statements <= statements, "{name}: {work:?}");
            assert_eq!(work.index_builds, hir.functions.len(), "{name}: {work:?}");
            if let Some(earlier) = previous {
                for (label, now, before) in [
                    ("blocks", work.blocks, earlier.blocks),
                    ("statements", work.statements, earlier.statements),
                    ("arguments", work.arguments, earlier.arguments),
                    ("access", work.access_incidences, earlier.access_incidences),
                    (
                        "selector",
                        work.selector_incidences,
                        earlier.selector_incidences,
                    ),
                    ("reach", work.reach_incidences, earlier.reach_incidences),
                    (
                        "compatibility",
                        work.compatibility_candidates,
                        earlier.compatibility_candidates,
                    ),
                    (
                        "support",
                        work.support_candidates,
                        earlier.support_candidates,
                    ),
                    ("reverse", work.reverse_visits, earlier.reverse_visits),
                ] {
                    let bound = if family == "receivers" && label != "arguments" {
                        before
                    } else {
                        4 * before
                    };
                    assert!(
                        now <= bound,
                        "{name}: {label} raw work grew from {before} to {now}"
                    );
                }
            }
            previous = Some(work);

            let named_index = |name: &str| &indexes[&named_function(&checker, name).id];
            match family {
                "scalar" => {
                    assert_eq!(guards, 0, "{name}");
                    for index in indexes.values() {
                        assert!(
                            index.ports.iter().all(|port| matches!(port,
                            super::EntryPort::Value { path, .. } if path.is_empty())),
                            "{name}"
                        );
                    }
                    assert_eq!(work.compatibility_candidates, 0, "{name}");
                    assert!(work.reverse_visits <= 100 * size, "{name}: {work:?}");
                }
                "independent" => {
                    let index = named_index("choose");
                    eprintln!(
                        "  choose: ports={} guarded_tests={} conditional_supports={} conditional_arms={} work={:?}",
                        index.ports.len(),
                        index.guards.len(),
                        index.conditional_supports.len(),
                        index.conditional_arm_count,
                        index.counts
                    );
                    assert_eq!(index.conditional_supports.len(), size, "{name}");
                    assert_eq!(index.conditional_arm_count, 2 * size, "{name}");
                    assert_eq!(index.counts.compatibility_candidates, 0, "{name}");
                    assert!(index.ports.len() <= 4 * size, "{name}");
                    assert!(
                        index.counts.support_candidates <= 16 * size,
                        "{name}: {:?}",
                        index.counts
                    );
                    assert!(
                        index.counts.reverse_visits <= 96 * size,
                        "{name}: {:?}",
                        index.counts
                    );
                }
                "correlated" => {
                    let index = named_index("change");
                    eprintln!(
                        "  change: ports={} guarded_tests={} conditional_supports={} work={:?}",
                        index.ports.len(),
                        index.guards.len(),
                        index.conditional_supports.len(),
                        index.counts
                    );
                    assert_eq!(index.guards.len(), 1, "{name}");
                    assert_eq!(index.conditional_supports.len(), 1, "{name}");
                    assert_eq!(index.counts.compatibility_candidates, size, "{name}");
                    assert!(index.ports.len() <= 2 * size + 8, "{name}");
                    assert!(
                        index.counts.support_candidates <= 24 * size,
                        "{name}: {:?}",
                        index.counts
                    );
                }
                "receivers" => {
                    let index = named_index("forward");
                    eprintln!(
                        "  forward: ports={} guarded_tests={} conditional_supports={} work={:?}",
                        index.ports.len(),
                        index.guards.len(),
                        index.conditional_supports.len(),
                        index.counts
                    );
                    assert_eq!(index.guards.len(), 1, "{name}");
                    assert!(index.ports.len() <= 8, "{name}");
                    assert!(
                        index.ports.iter().all(|port| match port {
                            super::EntryPort::Value { parameter, .. }
                            | super::EntryPort::Wrapper(parameter)
                            | super::EntryPort::Immediate(parameter)
                            | super::EntryPort::Terminal(parameter) => *parameter == 0,
                            super::EntryPort::Edge(..) => true,
                        }),
                        "{name}"
                    );
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn sparse_supports_couple_shared_control_without_merging_independent_diamonds() {
    let source = r#"
LeftHolder = |items {I32}|
RightHolder = |items {I32}|

shared_choice |left ~LeftHolder, right ~RightHolder, take Bool|:
    if take:
        ~left.items = {1}
        ~right.items = {2}
    ;
;

independent_choices |left ~LeftHolder, right ~RightHolder, take_left Bool, take_right Bool|:
    if take_left:
        ~left.items = {1}
    ;
    if take_right:
        ~right.items = {2}
    ;
;
"#;
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    let indices = build_sparse_index_for_test(&mut checker).unwrap();
    for (name, expected_supports) in [("shared_choice", 1), ("independent_choices", 2)] {
        let index = &indices[&named_function(&checker, name).id];
        let cell_supports: std::collections::BTreeSet<_> = index
            .ports
            .iter()
            .zip(&index.port_supports)
            .filter_map(|(port, support)| {
                matches!(port, super::EntryPort::Edge(..)).then_some(*support)
            })
            .collect();
        assert_eq!(cell_supports, index.conditional_supports, "{name}");
        assert_eq!(cell_supports.len(), expected_supports, "{name}");
    }
}

#[test]
fn sparse_collection_get_preserves_the_selected_tail_and_receiver() {
    let source = r#"
Item = |item {I32}, count I32|

read_item |items {Item}| -> {I32}:
    selected = items.get(0) catch:
        return {0}
    ;
    return selected.item
;

read_count |items {Item}| -> I32:
    selected = items.get(0) catch:
        return 0
    ;
    return selected.count
;
"#;
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    let indices = build_sparse_index_for_test(&mut checker).unwrap();
    for (name, field) in [("read_item", 0), ("read_count", 1)] {
        let index = &indices[&named_function(&checker, name).id];
        let path = |field| [CallField::Elem, CallField::Field(field)];
        let value = |expected: &[CallField]| {
            index.ports.iter().position(|port| {
                matches!(port, super::EntryPort::Value { parameter: 0, path } if path.as_ref() == expected)
            })
        };
        let selected = value(&path(field)).expect("selected member path");
        let receiver = value(&[]).expect("receiver loan root");
        assert!(value(&path(1 - field)).is_none());
        assert_eq!(index.port_supports[selected], index.port_supports[receiver]);
    }
}

#[test]
fn sparse_ordinary_call_results_preserve_a_selected_path_through_actuals() {
    let source = r#"
Saved = |item I32, count I32|
Envelope = |saved Saved|

relay |saved Saved| -> Saved:
    return saved
;

unwrap_saved |wrapped Envelope| -> Saved:
    return wrapped.saved
;

read_item |saved Saved| -> I32:
    forwarded = relay(saved)
    return forwarded.item
;

read_count |wrapped Envelope| -> I32:
    forwarded = unwrap_saved(wrapped)
    return forwarded.count
;
"#;
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    let indexes = build_sparse_index_for_test(&mut checker).unwrap();
    let field = CallField::Field;
    for (name, selected, sibling) in [
        ("read_item", &[field(0)][..], &[field(1)][..]),
        (
            "read_count",
            &[field(0), field(1)][..],
            &[field(0), field(0)][..],
        ),
    ] {
        let ports = &indexes[&named_function(&checker, name).id].ports;
        let has_path = |expected: &[CallField]| {
            ports.iter().any(|port| {
            matches!(port, super::EntryPort::Value { parameter: 0, path } if path.as_ref() == expected)
        })
        };
        assert!(has_path(selected), "{name}");
        assert!(!has_path(sibling), "{name}");
    }
}

#[test]
fn sparse_selected_results_follow_fresh_constructor_fields_to_the_right_actual() {
    let source = r#"
Saved = |item I32, count I32|
Envelope = |saved Saved, spare Saved|

wrap |saved Saved, spare Saved| -> Envelope:
    return Envelope(saved, spare)
;

relay |saved Saved, spare Saved| -> Envelope:
    forwarded = wrap(saved, spare)
    return forwarded
;

read_item |saved Saved, spare Saved| -> I32:
    forwarded = wrap(saved, spare)
    return forwarded.saved.item
;

read_count |saved Saved, spare Saved| -> I32:
    forwarded = relay(saved, spare)
    return forwarded.spare.count
;
"#;
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    let indexes = build_sparse_index_for_test(&mut checker).unwrap();
    for (name, parameter, selector) in [("read_item", 0, 0), ("read_count", 1, 1)] {
        let function = named_function(&checker, name);
        let projected: Vec<_> = indexes[&function.id]
            .ports
            .iter()
            .filter_map(|port| match port {
                super::EntryPort::Value { parameter, path } if !path.is_empty() => {
                    Some((*parameter, path.as_ref()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            projected,
            [(parameter, &[CallField::Field(selector)][..])],
            "{name}"
        );
    }
}

#[test]
fn sparse_supports_do_not_assume_an_ordinary_call_reconverges() {
    let source = r#"
LeftHolder = |items {I32}|
RightHolder = |items {I32}|

pending || -> I32:
    return pending()
;

choose |left ~LeftHolder, right ~RightHolder, take_left Bool, take_right Bool|:
    if take_left:
        ~left.items = {1}
        result = pending()
    ;
    if take_right:
        ~right.items = {2}
    ;
;
"#;
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    let indexes = build_sparse_index_for_test(&mut checker).unwrap();
    let function = named_function(&checker, "choose");
    let snapshot = super::Snapshot::collect(&checker, function).unwrap();
    assert!(
        snapshot
            .calls
            .iter()
            .any(|call| matches!(call.continuation, super::CallContinuation::Statement))
    );
    let index = &indexes[&function.id];
    let edits: std::collections::BTreeSet<_> = index
        .ports
        .iter()
        .zip(&index.port_supports)
        .filter_map(|(port, &support)| {
            matches!(port, super::EntryPort::Edge(..)).then_some(support)
        })
        .collect();
    assert_eq!(edits, index.conditional_supports);
    assert_eq!(edits.len(), 1);
}

#[test]
fn sparse_recursive_revisits_count_snapshot_rows_only_once() {
    let source = r#"
recur |take Bool, value I32| -> I32:
    if take:
        return value
    ;
    return recur(true, value)
;
"#;
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    let indices = build_sparse_index_for_test(&mut checker).unwrap();
    let function = named_function(&checker, "recur");
    let snapshot = super::Snapshot::collect(&checker, function).unwrap();
    let work = indices[&function.id].counts;
    assert!(work.index_builds > 1);
    assert_eq!(work.blocks, snapshot.counts.blocks);
    assert_eq!(work.statements, snapshot.counts.statements);
    assert_eq!(work.arguments, snapshot.counts.arguments);
}

fn with_results(source: &str, check: impl FnOnce(&dyn Fn(&str) -> FunctionId, &BorrowChecker<'_>)) {
    let fixture = ResultFixture::new(source);
    let mut checker = fixture.checker();
    checker.build_public_call_summaries().unwrap();
    let by_name = |name: &str| named_function(&checker, name).id;
    check(&by_name, &checker);
}

#[test]
fn cold_reverse_call_graph_includes_statement_invoke_and_never_edges() {
    with_results(
        r#"
ordinary |value I32| -> I32:
    return value
;

fallible |take Bool| -> I32, Error!:
    if take:
        return 1
    ;
    return! Error("no result")
;

stop || -> !:
    assert(false, "stop")
;

forward |take Bool| -> !:
    first = ordinary(0)
    value = fallible(take) catch then first
    stop()
;
"#,
        |function, checker| {
            let (_, reverse) = checker.cold_return_alias_indices().unwrap();
            let caller = function("forward");
            let caller_position = checker
                .module
                .functions
                .iter()
                .position(|function| function.id == caller)
                .unwrap();
            for callee in ["ordinary", "fallible", "stop"] {
                assert_eq!(reverse[&function(callee)], [caller_position], "{callee}");
            }
            let function = &checker.module.functions[caller_position];
            let blocks = checker.collect_reachable_blocks(function).unwrap();
            let terminal = blocks
                .iter()
                .map(|&id| checker.block_by_id_or_error(id, caller).unwrap())
                .find(|block| matches!(block.terminator, super::HirTerminator::NeverCall { .. }))
                .expect("forward ends with a terminal call");
            let mut successors = Vec::new();
            super::for_each_terminator_target(&terminal.terminator, |target| {
                successors.push(target)
            });
            assert!(successors.is_empty());
            let snapshot = super::Snapshot::collect(checker, function).unwrap();
            let cfg = super::SparseCfg::new(checker, &snapshot, &mut 0);
            for call in &snapshot.calls {
                let index = snapshot
                    .blocks
                    .iter()
                    .position(|id| id.0 == call.site.0)
                    .unwrap();
                let successors = &cfg.successors[index];
                assert!(successors.contains(&cfg.exit));
                match call.continuation {
                    super::CallContinuation::Invoke => assert_eq!(successors.len(), 3),
                    super::CallContinuation::Never => assert_eq!(successors, &[cfg.exit]),
                    super::CallContinuation::Statement => assert!(cfg.unproved_continuation[index]),
                }
            }
        },
    );
}

fn facts<'a>(checker: &'a BorrowChecker<'_>, function: FunctionId) -> &'a CallAllocationFacts {
    let FunctionReturnAliasSummary::Known(facts) =
        &checker.public_call_summaries[&function].return_alias
    else {
        panic!("local HIR should publish a known summary");
    };
    facts
}

fn result(slot: usize, path: &[CallField]) -> CallOutputPath {
    CallOutputPath::Result(CallResultPath {
        slot,
        path: path.to_vec(),
    })
}

fn param(index: usize, path: &[CallField]) -> CallParameterPath {
    CallParameterPath {
        index,
        path: path.to_vec(),
    }
}

fn has_source(
    lane: &CallLaneSummary,
    output: CallOutputPath,
    source: CallAllocationSource,
) -> bool {
    lane.sources
        .iter()
        .chain(&lane.stores)
        .any(|row| row.output == output && row.source == source)
}

#[test]
fn indexed_paths_preserve_slot_order_and_projected_overwrite() {
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

shared_view |items {I32}| -> Link:
    view = items
    return Link(view)
;

changed_field |items {I32}| -> Link:
    value ~= Link(items)
    writer ~= value.items
    ~writer = {7}
    return value
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

recursive |items {I32}| -> Link:
    return recursive(items)
;
"#,
        |function, checker| {
            let items = [CallField::Field(0)];
            let lane = |name| &facts(checker, function(name)).success;
            let parameter =
                |index, path: &[CallField]| CallAllocationSource::Parameter(param(index, path));
            assert!(has_source(
                lane("wrap"),
                result(0, &[]),
                CallAllocationSource::Fresh
            ));
            assert!(has_source(
                lane("wrap"),
                result(0, &items),
                parameter(0, &[])
            ));
            assert!(has_source(
                lane("wrap_copy"),
                result(0, &items),
                CallAllocationSource::Fresh
            ));
            assert!(has_source(
                lane("unwrap_link"),
                result(0, &[]),
                parameter(0, &items)
            ));
            assert!(has_source(
                lane("unwrap_temp"),
                result(0, &[]),
                parameter(0, &[])
            ));
            assert!(has_source(
                lane("ordered"),
                result(1, &[]),
                parameter(1, &[])
            ));
            assert!(has_source(
                lane("ordered"),
                result(2, &[]),
                CallAllocationSource::Fresh
            ));
            assert!(has_source(
                lane("changed_field"),
                result(0, &items),
                CallAllocationSource::Fresh
            ));
            assert!(!has_source(
                lane("changed_field"),
                result(0, &items),
                parameter(0, &[])
            ));
            assert!(has_source(
                lane("shared_view"),
                result(0, &items),
                parameter(0, &[])
            ));
            assert!(!lane("recursive").reachable);
        },
    );
}

#[test]
fn separate_exits_and_value_if_keep_same_execution_pairs_through_forwarding_and_copy() {
    for (body, shared_body) in [
        (
            "    if flag:\n        return Pair(a, {2})\n    ;\n    return Pair({3}, b)\n",
            "    if flag:\n        return Pair(a, b)\n    ;\n    return Pair({3}, b)\n",
        ),
        (
            "    chosen Pair = if flag:\n        then Pair(a, {2})\n    else\n        then Pair({3}, b)\n    ;\n    return Pair(chosen.left, chosen.right)\n",
            "    chosen Pair = if flag:\n        then Pair(a, b)\n    else\n        then Pair({3}, b)\n    ;\n    return Pair(chosen.left, chosen.right)\n",
        ),
    ] {
        for (body, shares) in [(body, false), (shared_body, true)] {
            let source = format!(
                r#"
Pair = |left {{I32}}, right {{I32}}|

forward |flag Bool, items {{I32}}| -> Pair:
    selected = select(flag, items, items)
    return copy selected
;

select |flag Bool, a {{I32}}, b {{I32}}| -> Pair:
{body};
"#
            );
            with_results(&source, |function, checker| {
                let left = result(0, &[CallField::Field(0)]);
                let right = result(0, &[CallField::Field(1)]);
                let select = &facts(checker, function("select")).success;
                let forwarded = &facts(checker, function("forward")).success;
                assert!(has_source(
                    select,
                    left.clone(),
                    CallAllocationSource::Parameter(param(1, &[]))
                ));
                assert!(has_source(
                    select,
                    right.clone(),
                    CallAllocationSource::Parameter(param(2, &[]))
                ));
                let relation = |condition| CallSharing {
                    left: left.clone(),
                    right: right.clone(),
                    condition,
                };
                assert_eq!(
                    select
                        .sharing
                        .contains(&relation(CallSharingCondition::Parameters(
                            param(1, &[]),
                            param(2, &[])
                        ))),
                    shares
                );
                assert_eq!(
                    forwarded
                        .sharing
                        .contains(&relation(CallSharingCondition::Always)),
                    shares
                );
                assert!(has_source(forwarded, left, CallAllocationSource::Fresh));
                assert!(has_source(forwarded, right, CallAllocationSource::Fresh));
            });
        }
    }
}

#[test]
fn nested_actual_parameter_paths_substitute_without_root_widening() {
    with_results(
        r#"
Pair = |left {I32}, right {I32}|
Outer = |pair Pair|

make_pair |left {I32}, right {I32}| -> Pair:
    return Pair(left, right)
;

forward |outer Outer| -> Pair:
    return make_pair(outer.pair.left, outer.pair.right)
;
"#,
        |function, checker| {
            let a = &[CallField::Field(0), CallField::Field(0)];
            let b = &[CallField::Field(0), CallField::Field(1)];
            let left = result(0, &[CallField::Field(0)]);
            let right = result(0, &[CallField::Field(1)]);
            let lane = &facts(checker, function("forward")).success;
            assert!(has_source(
                lane,
                left.clone(),
                CallAllocationSource::Parameter(param(0, a))
            ));
            assert!(has_source(
                lane,
                right.clone(),
                CallAllocationSource::Parameter(param(0, b))
            ));
            assert_eq!(
                lane.sharing,
                [CallSharing {
                    left,
                    right,
                    condition: CallSharingCondition::Parameters(param(0, a), param(0, b)),
                }]
            );
        },
    );
}

#[test]
fn pairs_from_different_exits_do_not_become_transitive_after_substitution() {
    with_results(
        r#"
Triple = |left {I32}, middle {I32}, right {I32}|

select |flag Bool, a {I32}, b {I32}, c {I32}, d {I32}| -> Triple:
    if flag:
        return Triple(a, b, {3})
    ;
    return Triple({1}, c, d)
;

forward |flag Bool, items {I32}, others {I32}| -> Triple:
    selected = select(flag, items, items, others, others)
    return copy selected
;
"#,
        |function, checker| {
            let left = result(0, &[CallField::Field(0)]);
            let middle = result(0, &[CallField::Field(1)]);
            let right = result(0, &[CallField::Field(2)]);
            let selected = &facts(checker, function("select")).success;
            assert_eq!(
                selected.sharing,
                [
                    CallSharing {
                        left: left.clone(),
                        right: middle.clone(),
                        condition: CallSharingCondition::Parameters(param(1, &[]), param(2, &[])),
                    },
                    CallSharing {
                        left: middle.clone(),
                        right: right.clone(),
                        condition: CallSharingCondition::Parameters(param(3, &[]), param(4, &[])),
                    },
                ]
            );
            let forwarded = &facts(checker, function("forward")).success;
            assert_eq!(
                forwarded.sharing,
                [
                    CallSharing {
                        left,
                        right: middle.clone(),
                        condition: CallSharingCondition::Always
                    },
                    CallSharing {
                        left: middle,
                        right,
                        condition: CallSharingCondition::Always
                    },
                ]
            );
        },
    );
}

#[test]
fn stores_are_lane_specific_and_catch_projects_failure_stores_to_its_own_success() {
    with_results(
        r#"
fail_after_write |rows ~{{I32}}, child {I32}, ok Bool| -> I32, Error!:
    if ok:
        return 7
    ;
    ~rows.push(child)
    return! Error("failed")
;

write_on_success |rows ~{{I32}}, child {I32}, ok Bool| -> I32, Error!:
    if ok:
        ~rows.push(child)
        return 7
    ;
    return! Error("failed")
;

caught |rows ~{{I32}}, child {I32}, ok Bool| -> I32:
    status = fail_after_write(~rows, child, ok) catch then 0
    return status
;
"#,
        |function, checker| {
            let target = CallOutputPath::Store {
                target: param(0, &[CallField::Elem]),
                path: Vec::new(),
            };
            let source = CallAllocationSource::Parameter(param(1, &[]));
            let failure = facts(checker, function("fail_after_write"));
            assert!(failure.success.reachable && failure.error.reachable);
            assert!(failure.success.stores.is_empty());
            assert!(has_source(&failure.error, target.clone(), source.clone()));
            let success = facts(checker, function("write_on_success"));
            assert!(has_source(&success.success, target.clone(), source.clone()));
            assert!(success.error.stores.is_empty());
            let caught = facts(checker, function("caught"));
            assert!(has_source(&caught.success, target, source));
            assert!(!caught.error.reachable);
        },
    );
}

#[test]
fn captured_option_payload_and_new_stored_subtree_keep_distinct_store_coordinates() {
    use super::Lane;
    use super::graph::{Cell, Graph, Origin, Owner, Site, Spec};
    use crate::compiler_frontend::datatypes::ids::TypeId;

    let mut graph = Graph::default();
    let payload = CallField::Payload {
        variant: 1,
        field: 0,
    };
    let field = CallField::Field(0);
    let carrier = Cell::root(Owner::Local(0));
    let previous_items = Cell::root(Owner::Local(1));
    let new_items = Cell::root(Owner::Local(2));
    let replacement = Cell::root(Owner::Local(3));
    let parameter = |cell, ty, index, path: &[CallField]| {
        Spec::born(cell, TypeId(ty), Origin::Parameter(param(index, path)))
    };
    graph.install(
        vec![
            parameter(carrier.clone(), 20, 0, &[]),
            parameter(carrier.at(&[payload]), 21, 0, &[payload]),
            parameter(carrier.at(&[payload, field]), 22, 0, &[payload, field]),
            parameter(previous_items.clone(), 22, 1, &[]),
            parameter(new_items.clone(), 22, 2, &[]),
            Spec::born(
                replacement.clone(),
                TypeId(21),
                Origin::Fresh(Site::Value(0), 0, Vec::new()),
            ),
        ],
        &[],
        None,
    );
    let original = graph.capture(&carrier.at(&[payload]));
    assert!(graph.write(&carrier, payload, &replacement, false, |_| true));
    assert!(graph.write(&original, field, &previous_items, false, |_| true));
    assert!(graph.write(&replacement, field, &new_items, false, |_| true));
    let mut lane = Lane::default();
    lane.extend(&graph, &[replacement]);
    let lane = lane.finish();
    assert!(has_source(
        &lane,
        CallOutputPath::Store {
            target: param(0, &[payload, field]),
            path: Vec::new()
        },
        CallAllocationSource::Parameter(param(1, &[])),
    ));
    assert!(has_source(
        &lane,
        CallOutputPath::Store {
            target: param(0, &[payload]),
            path: vec![field]
        },
        CallAllocationSource::Parameter(param(2, &[])),
    ));
}

#[test]
fn fresh_value_shared_between_a_result_and_a_projected_store_has_an_explicit_pair() {
    with_results(
        r#"
Link = |items {I32}|
Outer = |link Link|

replace |outer ~Outer, items {I32}| -> Link:
    fresh ~= Link(items)
    ~outer.link = fresh
    return fresh
;

through_call |outer ~Outer, items {I32}| -> Link:
    return replace(~outer, items)
;
"#,
        |function, checker| {
            let field = CallField::Field(0);
            let store = CallOutputPath::Store {
                target: param(0, &[field]),
                path: Vec::new(),
            };
            let relative = CallOutputPath::Store {
                target: param(0, &[field]),
                path: vec![field],
            };
            for name in ["replace", "through_call"] {
                let lane = &facts(checker, function(name)).success;
                assert!(has_source(lane, store.clone(), CallAllocationSource::Fresh));
                assert!(has_source(
                    lane,
                    relative.clone(),
                    CallAllocationSource::Parameter(param(1, &[]))
                ));
                assert!(lane.sharing.contains(&CallSharing {
                    left: result(0, &[]),
                    right: store.clone(),
                    condition: CallSharingCondition::Always,
                }));
            }
        },
    );
}

#[test]
fn only_projected_writes_reachable_from_a_result_change_its_sources() {
    with_results(
        r#"
Link = |items {I32}|
Outer = |link Link|

unreturned_scratch |incoming {I32}| -> Link:
    child ~= {0}
    scratch ~= Link(child)
    ~scratch.items = incoming
    return Link(child)
;

retained_holder |incoming {I32}| -> Link:
    holder ~= Link({0})
    scratch ~= Outer(holder)
    ~scratch.link.items = incoming
    return holder
;
"#,
        |function, checker| {
            let items = result(0, &[CallField::Field(0)]);
            let param = CallAllocationSource::Parameter(param(0, &[]));
            let scratch = &facts(checker, function("unreturned_scratch")).success;
            assert!(has_source(
                scratch,
                items.clone(),
                CallAllocationSource::Fresh
            ));
            assert!(!has_source(scratch, items.clone(), param.clone()));
            let retained = &facts(checker, function("retained_holder")).success;
            assert!(has_source(retained, items, param));
        },
    );
}

#[test]
fn optional_wrap_keeps_only_the_proven_payload_provenance() {
    let retained = include_str!(
        "../../../../../tests/cases/runtime_call_opaque_option_retained_result_rejected/input/@entry.moth"
    );
    let fresh = include_str!(
        "../../../../../tests/cases/runtime_call_opaque_option_fresh_result/input/@entry.moth"
    );
    let items = result(0, &[CallField::Field(0)]);
    let payload = CallAllocationSource::Parameter(param(
        0,
        &[CallField::Payload {
            variant: 1,
            field: 0,
        }],
    ));
    with_results(retained, |function, checker| {
        let lane = &facts(checker, function("wrap_option")).success;
        assert!(has_source(lane, items.clone(), payload.clone()));
        assert!(has_source(lane, items.clone(), CallAllocationSource::Fresh));
    });
    with_results(fresh, |function, checker| {
        let lane = &facts(checker, function("fresh_wrap_option")).success;
        assert!(has_source(lane, items.clone(), CallAllocationSource::Fresh));
        assert!(!has_source(lane, items, payload));
    });
}

#[test]
fn returned_option_payload_keeps_the_caller_argument_borrowed() {
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
            assert!(has_source(
                &facts(checker, unwrap_link).success,
                result(0, &[]),
                CallAllocationSource::Parameter(param(
                    0,
                    &[CallField::Payload {
                        variant: 1,
                        field: 0
                    },]
                ))
            ));

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
            let report = run_borrow_checker_with_types(
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
                    .unwrap()
                    .optional_transfer,
                OptionalTransferStatus::Borrow
            );
        },
    );
}

// A recursive callee first publishes only its `none` exit. Its `Some(holder)` exit appears
// after its own summary becomes known, which resolves the caller's payload holder.
const RECURSIVE_PICK: &str = r#"
pick |again Bool, holder ~Link| -> Link?:
    if again:
        if pick(false, ~holder) is none:
            return holder
        ;
    ;
    return none
;
"#;

const WRITE_THROUGH_PICK: &str = r#"
through |holder ~Link, first {I32}, next {I32}, flag Bool| -> Link?:
    writer ~= pick(flag, ~holder)?
    ~holder.items = first
    ~writer.items = next
    return writer
;
"#;

const MUTUAL_PING: &str = r#"
ping |depth I32, holder ~Link| -> Link?:
    if depth > 0:
        return pong(depth - 1, ~holder)
    ;
    return none
;
"#;

const MUTUAL_PONG: &str = r#"
pong |depth I32, holder ~Link| -> Link?:
    if depth is 1:
        return holder
    ;
    return ping(depth, ~holder)
;
"#;

const WRITE_THROUGH_PING: &str = r#"
relay |holder ~Link, next {I32}| -> Link?:
    writer ~= ping(3, ~holder)?
    ~writer.items = next
    return writer
;
"#;

fn recursive_module(declarations: &[&str]) -> String {
    let mut source = String::from("Link = |items {I32}|\n");
    for declaration in declarations {
        source.push_str(declaration);
    }
    source
}

#[test]
fn a_write_through_a_holder_a_recursive_callee_resolves_later_converges_monotonically() {
    // Before the fix, the first pass published `first` because the unresolved payload write was
    // dropped. The resolved write is strong, so publication then narrowed and was rejected.
    with_results(
        &recursive_module(&[RECURSIVE_PICK, WRITE_THROUGH_PICK]),
        |function, checker| {
            let lane = &facts(checker, function("through")).success;
            let items = CallOutputPath::Store {
                target: param(0, &[CallField::Field(0)]),
                path: Vec::new(),
            };
            assert!(has_source(
                lane,
                items.clone(),
                CallAllocationSource::Parameter(param(2, &[]))
            ));
            assert!(!has_source(
                lane,
                items,
                CallAllocationSource::Parameter(param(1, &[]))
            ));
            let payload = CallField::Payload {
                variant: 1,
                field: 0,
            };
            assert!(has_source(
                lane,
                result(0, &[payload]),
                CallAllocationSource::Parameter(param(0, &[]))
            ));
        },
    );
}

#[test]
fn recursive_summaries_reach_the_same_fixed_point_across_definition_orders() {
    // Definition order is the worklist's initial order, and it also decides which caller is
    // re-queued first when a recursive callee widens.
    let declarations = [
        RECURSIVE_PICK,
        WRITE_THROUGH_PICK,
        MUTUAL_PING,
        MUTUAL_PONG,
        WRITE_THROUGH_PING,
    ];
    let names = ["pick", "through", "ping", "pong", "relay"];
    let mut orders = Vec::new();
    for rotation in 0..declarations.len() {
        let mut order = declarations.to_vec();
        order.rotate_left(rotation);
        orders.push(order.clone());
        order.reverse();
        orders.push(order);
    }
    let mut expected = None;
    for order in orders {
        with_results(&recursive_module(&order), |function, checker| {
            let summaries =
                names.map(|name| checker.public_call_summaries[&function(name)].clone());
            match &expected {
                None => expected = Some(summaries),
                Some(expected) => assert_eq!(expected, &summaries),
            }
        });
    }
    let expected = expected.unwrap();
    let relay = match &expected[4].return_alias {
        FunctionReturnAliasSummary::Known(facts) => &facts.success,
        other => panic!("relay should publish a known summary, got {other:?}"),
    };
    let items = CallOutputPath::Store {
        target: param(0, &[CallField::Field(0)]),
        path: Vec::new(),
    };
    assert!(has_source(
        relay,
        items,
        CallAllocationSource::Parameter(param(1, &[]))
    ));
}

#[test]
fn a_definite_mutation_through_an_absent_receiver_does_not_publish_later_stores() {
    // `pick` never yields a value, so everything after `?` is impossible. A push is a weak
    // element update and a clear an erase, but both still need their receiver: skipping either
    // would publish the `rows` store from a path that cannot run.
    for (holder, mutation) in [
        ("{{I32}}", "~holder.push(child)"),
        ("{String = {I32}}", "~holder.clear()"),
    ] {
        with_results(
            &format!(
                r#"
pick |flag Bool| -> {holder}?:
    return none
;

stash |rows ~{{{{I32}}}}, child {{I32}}, flag Bool| -> {holder}?:
    holder ~= pick(flag)?
    {mutation}
    ~rows.push(child)
    return none
;
"#
            ),
            |function, checker| {
                let lane = &facts(checker, function("stash")).success;
                assert!(
                    lane.reachable,
                    "{mutation}: the propagated `none` exit still succeeds"
                );
                assert!(
                    lane.stores.is_empty(),
                    "{mutation}: no store may come from the impossible path: {:?}",
                    lane.stores
                );
            },
        );
    }
}

#[test]
fn a_slot_assigned_a_place_writes_through_it_like_a_defined_alias() {
    // `~writer = box.items` turns the plain slot into an alias, so the next assignment stores
    // `incoming` into `box.items`. Treating it as a rebind would publish the entry items.
    with_results(
        r#"
Box = |items {I32}|

stash |box ~Box, incoming {I32}| -> {I32}:
    writer ~= {0}
    ~writer = box.items
    ~writer = incoming
    size = writer.length()
    return box.items
;

stash_after_self_update |box ~Box, incoming {I32}| -> {I32}:
    writer ~= {0}
    ~writer = writer
    ~writer = box.items
    ~writer = incoming
    size = writer.length()
    return box.items
;
"#,
        |function, checker| {
            let parameter =
                |index, path: &[CallField]| CallAllocationSource::Parameter(param(index, path));
            // A direct self-update keeps the plain slot, so the later place load still aliases.
            for name in ["stash", "stash_after_self_update"] {
                let lane = &facts(checker, function(name)).success;
                assert!(
                    has_source(lane, result(0, &[]), parameter(1, &[])),
                    "{name}"
                );
                assert!(
                    !has_source(lane, result(0, &[]), parameter(0, &[CallField::Field(0)])),
                    "{name}"
                );
            }
        },
    );
}

#[test]
fn projected_aliases_acquired_on_different_branches_keep_their_own_selectors() {
    // The two acquisitions select `items` at different ordinals. Merging their captured
    // holders would apply each selector to both holders and store heap data into scalar `pad`.
    with_results(
        r#"
Left = |items {I32}, pad I32|
Right = |pad I32, items {I32}|

stash |flag Bool, left ~Left, right ~Right| -> I32:
    writer ~= {0}
    if flag:
        ~writer = left.items
    else
        ~writer = right.items
    ;
    ~writer = {7}
    return writer.length()
;
"#,
        |function, checker| {
            let lane = &facts(checker, function("stash")).success;
            let store = |index, field| CallOutputPath::Store {
                target: param(index, &[CallField::Field(field)]),
                path: Vec::new(),
            };
            assert!(has_source(lane, store(1, 0), CallAllocationSource::Fresh));
            assert!(has_source(lane, store(2, 1), CallAllocationSource::Fresh));
            assert!(
                lane.stores
                    .iter()
                    .all(|row| row.output != store(1, 1) && row.output != store(2, 0))
            );
        },
    );
}

const SLOT_OR_ALIAS_STASH: &str = r#"
stash |flag Bool, left ~Left, right ~Right, incoming {I32}| -> {I32}:
    writer ~= {0}
    if flag:
        ready = gate()
        ~writer = left.items
    else
        ~writer = {0}
    ;
    ~writer = right.items
    ~writer = identity(incoming)
    size = writer.length()
    return right.items
;
"#;

#[test]
fn a_joined_slot_alternative_still_acquires_an_assigned_alias_in_every_order() {
    // After the join `writer` is a slot on one path and an alias of `left.items` on the other.
    // The slot path aliases `right.items` and stores `incoming` there; the alias path writes
    // through `left.items`. `gate` resolves late in some orders, so the slot path is first seen
    // alone, and the final summary must not depend on that.
    let declarations = [
        "gate || -> I32:\n    return seed()\n;\n",
        "seed || -> I32:\n    return 1\n;\n",
        "identity |value {I32}| -> {I32}:\n    return value\n;\n",
        SLOT_OR_ALIAS_STASH,
    ];
    let mut expected = None;
    for rotation in 0..declarations.len() {
        for reverse in [false, true] {
            let mut order = declarations.to_vec();
            order.rotate_left(rotation);
            if reverse {
                order.reverse();
            }
            let source = format!(
                "Left = |items {{I32}}|\nRight = |items {{I32}}|\n\n{}",
                order.join("\n")
            );
            with_results(&source, |function, checker| {
                let summary = checker.public_call_summaries[&function("stash")].clone();
                match &expected {
                    None => expected = Some(summary),
                    Some(expected) => assert_eq!(expected, &summary),
                }
            });
        }
    }
    let FunctionReturnAliasSummary::Known(facts) = &expected.unwrap().return_alias else {
        panic!("stash should publish a known summary");
    };
    let parameter = |index, path: &[CallField]| CallAllocationSource::Parameter(param(index, path));
    assert!(has_source(
        &facts.success,
        result(0, &[]),
        parameter(3, &[])
    ));
    assert!(has_source(
        &facts.success,
        result(0, &[]),
        parameter(2, &[CallField::Field(0)])
    ));
}

#[test]
fn a_bottom_alias_alternative_does_not_discard_the_slot_alternative() {
    // `found` never exists, so `writer`'s alias alternative is impossible. The slot alternative
    // still aliases `box.items` and stores `incoming` there.
    with_results(
        r#"
Box = |items {I32}|

nothing || -> Box?:
    return none
;

stash |flag Bool, box ~Box, incoming {I32}| -> {I32}:
    writer ~= {0}
    if flag:
        maybe = nothing()
        if maybe is |found|:
            ~writer = found.items
        ;
    ;
    ~writer = box.items
    ~writer = incoming
    size = writer.length()
    return box.items
;
"#,
        |function, checker| {
            let lane = &facts(checker, function("stash")).success;
            assert!(lane.reachable);
            assert!(has_source(
                lane,
                result(0, &[]),
                CallAllocationSource::Parameter(param(2, &[]))
            ));
        },
    );
}

const PENDING_ALIAS_STASH: &str = r#"
stash |flag Bool, delay Bool, target ~Holder, child {I32}| -> Holder?:
    writer ~= {0}
    if flag:
        missing ~= pick()?
        ~writer = missing.items
    else
        if delay:
            ~writer = {1}
        else
            ~writer = {2}
        ;
    ;
    ~writer = target.items
    ~writer = child
    return target
;
"#;

#[test]
fn a_resolved_bottom_alias_alternative_keeps_the_published_slot_store() {
    // While `pick` is pending, only the slot arm reaches the join and publishes the store. When
    // `pick` resolves to `none`, the bottom alias arm reaches it too and must not retract it.
    let pick = "pick || -> Holder?:\n    return none\n;\n";
    for order in [[pick, PENDING_ALIAS_STASH], [PENDING_ALIAS_STASH, pick]] {
        let source = format!("Holder = |items {{I32}}|\n\n{}", order.join("\n"));
        with_results(&source, |function, checker| {
            let lane = &facts(checker, function("stash")).success;
            let items = CallOutputPath::Store {
                target: param(2, &[CallField::Field(0)]),
                path: Vec::new(),
            };
            assert!(has_source(
                lane,
                items,
                CallAllocationSource::Parameter(param(3, &[]))
            ));
        });
    }
}

#[test]
fn elements_captured_from_one_fresh_collection_atom_are_not_must_aliases() {
    // Both captures carry the single `Fresh` atom at `Elem`, which stands for every element.
    // Replacing one capture's field must not strongly kill the other capture's items.
    with_results(
        r#"
Box = |items {I32}|

build || -> {Box}:
    return {Box({1}), Box({2})}
;

probe |incoming {I32}| -> {I32}:
    boxes ~= build()
    first ~= boxes.get(0) catch:
        return incoming
    ;
    second = boxes.get(1) catch:
        return incoming
    ;
    ~first.items = incoming
    return second.items
;
"#,
        |function, checker| {
            // The catch exits return `incoming`, so `Fresh` can only come from the normal exit.
            let lane = &facts(checker, function("probe")).success;
            assert!(has_source(
                lane,
                result(0, &[]),
                CallAllocationSource::Fresh
            ));
        },
    );
}

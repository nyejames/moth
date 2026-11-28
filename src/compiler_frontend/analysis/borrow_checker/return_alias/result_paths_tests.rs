use super::{BorrowChecker, IndexCounts, SparseFunction, build_sparse_index_for_test};
use crate::compiler_frontend::analysis::borrow_checker::OptionalTransferStatus;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::public_call_summary::typed::{
    CellPort, CellPost, EntryPort, ExportPort, FamilyPort, FamilyPost, Lane, ObjectRef, PortId,
    Post, TypedCallAllocationFacts, ValueSource,
};
use crate::compiler_frontend::public_call_summary::{CallField, FunctionReturnAliasSummary};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{PreparedHirFixture, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use rustc_hash::FxHashMap;
use std::convert::Infallible;
use std::path::Path;
use std::time::Instant;

#[test]
fn entry_identity_presentations_respect_transitive_equality() {
    use super::{allocation_classes, visit_entry_identities};
    use crate::compiler_frontend::public_call_summary::typed::{IdentityKind, Predicate};

    let [a, b, c] = [PortId(0), PortId(1), PortId(2)];
    let equal = |left, right| Predicate::Equal(IdentityKind::Allocation, left, right);
    let mut presentations = Vec::new();
    visit_entry_identities(
        &[equal(a, b), equal(a, c), equal(b, c)],
        &mut Vec::new(),
        &mut |literals, _| {
            let classes = allocation_classes(literals);
            let root = |port| classes.get(&port).copied().unwrap_or(port);
            presentations.push([root(a), root(b), root(c)]);
            Ok::<_, Infallible>(())
        },
    )
    .unwrap();
    presentations.sort_unstable();
    assert_eq!(
        presentations,
        [[a, a, a], [a, a, c], [a, b, a], [a, b, b], [a, b, c]],
    );
}

struct ResultFixture(PreparedHirFixture);

impl ResultFixture {
    fn new(source: &str) -> Self {
        Self(PreparedHirFixture::new(source))
    }

    fn checker(&self) -> BorrowChecker<'_> {
        BorrowChecker::new(
            &self.0.hir,
            &self.0.hir.type_environment,
            &self.0.packages,
            &self.0.paths,
            &self.0.strings,
        )
        .with_type_projection(&self.0.type_projection())
    }

    fn indexed(&self) -> (BorrowChecker<'_>, FxHashMap<FunctionId, SparseFunction>) {
        let mut checker = self.checker();
        let indexes = build_sparse_index_for_test(&mut checker).expect("sparse index");
        (checker, indexes)
    }

    fn with_results(&self, check: impl FnOnce(&dyn Fn(&str) -> FunctionId, &BorrowChecker<'_>)) {
        let mut checker = self.checker();
        checker.build_public_call_summaries().unwrap();
        let by_name = |name: &str| named_function(&checker, name).id;
        check(&by_name, &checker);
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
fn sparse_forward_transfer_executes_admitted_entry_roles_without_validation() {
    use crate::compiler_frontend::analysis::borrow_checker::state::{
        AddressSet, BindingSpec, BindingTarget, CellId, Coverage, FieldKey, RootSet, WrapperId,
    };
    use crate::compiler_frontend::public_call_summary::typed::Role;

    let fixture = ResultFixture::new(
        r#"
probe |dest ~I32, source I32, unrelated I32|:
    ~dest = source
    ignored = unrelated
;
"#,
    );
    let (checker, indexes) = fixture.indexed();
    let function = named_function(&checker, "probe");
    let index = &indexes[&function.id];
    let support = index.operations[&(function.entry.0, 0)][0];
    let projected_support = index.project_support(support, [], []);
    let block_view = projected_support.block_view(function.entry);
    assert_eq!(block_view.statement_positions, [0]);
    let view = block_view.operations;
    assert!(view.index_of(function.params[2]).is_none());
    let dest = view
        .index_of(function.params[0])
        .expect("destination index");
    let source = view.index_of(function.params[1]).expect("source index");
    let source_wrapper = WrapperId::current(source);
    for role in [Role::Slot, Role::WholeAlias, Role::ProjectedAlias] {
        let mut entry = projected_support.seed_entry([(function.params[1], BindingSpec::Slot)]);
        let projected = entry.local_alloc_ids(source).0[0];
        let key = FieldKey::Field(0);
        let binding = match role {
            Role::Slot => BindingSpec::Slot,
            Role::WholeAlias => BindingSpec::Link {
                source: source_wrapper,
                writable: true,
            },
            Role::ProjectedAlias => BindingSpec::Projected {
                addresses: AddressSet::from_cells(
                    [CellId::Projected(projected, key)],
                    Coverage::Complete,
                )
                .with_singleton_proof(),
                origin: source_wrapper,
                owner: None,
                writable: true,
                isolated: true,
                route_roots: RootSet::empty(view.local_count()),
            },
        };
        entry.initialize_entry_binding(dest, binding);
        let exits = checker
            .drive_sparse_support(function, entry, &projected_support)
            .expect("the transfer must not use the borrow verdict");
        let after = exits[&function.entry].boundary_binding(WrapperId::current(dest));
        let expected = match role {
            Role::Slot | Role::WholeAlias => BindingTarget::Link(source_wrapper),
            Role::ProjectedAlias => BindingTarget::Projected(projected, key),
        };
        assert_eq!(
            after
                .alternatives
                .iter()
                .map(|alt| alt.target)
                .collect::<Vec<_>>(),
            [expected]
        );
    }
}

#[test]
fn sparse_forward_skips_unrelated_diamonds() {
    use crate::compiler_frontend::analysis::borrow_checker::state::BindingSpec;
    use std::collections::BTreeSet;

    const DIAMONDS: usize = 16;
    let mut source = String::new();
    for i in 0..DIAMONDS {
        source.push_str(&format!("Holder{i} = |items {{I32}}|\n"));
    }
    let params = (0..DIAMONDS)
        .flat_map(|i| [format!("target_{i} ~Holder{i}"), format!("take_{i} Bool")])
        .collect::<Vec<_>>()
        .join(", ");
    source.push_str(&format!("\nchoose |{params}|:\n"));
    for i in 0..DIAMONDS {
        source.push_str(&format!(
            "    if take_{i}:\n        ~target_{i}.items = {{2, 3}}\n    ;\n"
        ));
    }
    source.push_str(";\n");
    let fixture = ResultFixture::new(&source);
    let (checker, indexes) = fixture.indexed();
    let function = named_function(&checker, "choose");
    let index = &indexes[&function.id];
    let supports: BTreeSet<_> = index.operations.values().flatten().copied().collect();
    assert_eq!(supports.len(), DIAMONDS);
    for support in supports {
        let projected = index.project_support(support, [], []);
        let entry = projected.seed_entry(function.params.iter().copied().filter_map(|param| {
            projected
                .local_indices
                .contains_key(&param)
                .then_some((param, BindingSpec::Slot))
        }));
        let exits = checker
            .drive_sparse_support(function, entry, &projected)
            .expect("transfer of one independent support");
        assert!(
            exits
                .values()
                .all(|state| state.shares_test_recurring_index(&index.transfer.recurring))
        );
        assert!(
            exits.len() <= 4,
            "visited {} blocks for one diamond",
            exits.len()
        );
    }
}

#[test]
fn sparse_compaction_preserves_the_join_before_visibility_narrowing() {
    use crate::compiler_frontend::analysis::borrow_checker::state::{
        BindingSpec, RootSet, ValueKind,
    };

    let fixture = ResultFixture::new(
        r#"
Holder = |items {I32}|
choose |target ~Holder, take Bool, step Bool, hidden I32, entry_only I32, incoming {I32}|:
    if take:
        ~target.items = incoming
    ;
    if step:
        ignored = 0
    ;
;
"#,
    );
    let (checker, indexes) = fixture.indexed();
    let function = named_function(&checker, "choose");
    let index = &indexes[&function.id];
    let coordinates = &index.transfer;
    let (&(write_block, write_position), supports) = index.operations.iter()
        .find(|&(&(block, statement), supports)| {
            !supports.is_empty() && checker.module.blocks[checker.block_index_by_id[&super::BlockId(block)]]
                .statements.get(statement).is_some_and(|statement| matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: crate::compiler_frontend::hir::statements::HirWriteTarget::AssignPlace(place), ..
                    } if !place.projections.is_empty()
                ))
        }).expect("selected inner-arm write");
    let write_block = super::BlockId(write_block);
    let support = supports[0];
    let (&merge, _) = coordinates
        .predecessors
        .iter()
        .filter(|&(block, predecessors)| {
            predecessors.len() > 1 && !coordinates.successors[block].is_empty()
        })
        .max_by_key(|&(block, _)| coordinates.ranks[block])
        .expect("merge before a continuation");
    let target = coordinates.successors[&merge][0];
    let without = |local| {
        let mut roots = RootSet::full(coordinates.local_ids.len());
        roots.remove(coordinates.local_indices[&local]);
        roots
    };
    let mut projected = index.project_support(
        support,
        function.params.iter().copied(),
        [
            (function.entry, without(function.params[4])),
            (target, without(function.params[3])),
        ],
    );
    assert!(
        projected
            .block_view(write_block)
            .statement_positions
            .contains(&write_position),
        "the support must execute the projected write"
    );
    assert_eq!(
        projected.continuation(merge),
        merge,
        "the merge must precede narrowing"
    );
    let entry = projected.seed_entry(
        function
            .params
            .iter()
            .copied()
            .map(|local| (local, BindingSpec::Slot)),
    );
    let holder = entry
        .local_alloc_ids(projected.local_indices[&function.params[0]])
        .0[0];
    // Loading an existing collection records an edge even when the layer starts empty.
    let incoming = entry
        .local_alloc_ids(projected.local_indices[&function.params[5]])
        .0[0];
    let compressed = checker
        .drive_sparse_support(function, entry.clone(), &projected)
        .expect("compressed transfer");
    projected.compress = false;
    let authored = checker
        .drive_sparse_support(function, entry, &projected)
        .expect("authored transfer");
    for (&block, state) in &compressed {
        assert_eq!(state, &authored[&block], "different facts at {block:?}");
    }
    let hidden = projected.local_indices[&function.params[3]];
    let entry_only = projected.local_indices[&function.params[4]];
    assert!(
        compressed[&function.entry]
            .binding_mode(entry_only)
            .is_definitely_uninit()
    );
    assert!(
        compressed[&target]
            .binding_mode(hidden)
            .is_definitely_uninit()
    );
    // `Holder` has one field, so any retained edge from the holder is `items`. Field keys are
    // interned field identities, not positions.
    let holds_incoming = |block| {
        compressed[&block]
            .retained_edge_groups(holder)
            .flatten()
            .any(|edge| {
                matches!(edge.value_alternative().kind,
                    ValueKind::Allocation(value) if value.id == incoming)
            })
    };
    assert!(holds_incoming(write_block));
    assert!(holds_incoming(target));
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
    let (checker, indices) = fixture.indexed();
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
    let (checker, indices) = fixture.indexed();
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
    let (checker, indexes) = fixture.indexed();
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
    let (checker, indexes) = fixture.indexed();
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
    let (checker, indexes) = fixture.indexed();
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
    let (checker, indices) = fixture.indexed();
    let function = named_function(&checker, "recur");
    let snapshot = super::Snapshot::collect(&checker, function).unwrap();
    let work = indices[&function.id].counts;
    assert!(work.index_builds > 1);
    assert_eq!(work.blocks, snapshot.counts.blocks);
    assert_eq!(work.statements, snapshot.counts.statements);
    assert_eq!(work.arguments, snapshot.counts.arguments);
}

fn with_results(source: &str, check: impl FnOnce(&dyn Fn(&str) -> FunctionId, &BorrowChecker<'_>)) {
    ResultFixture::new(source).with_results(check);
}

#[test]
fn cold_dependency_index_includes_statement_invoke_and_never_edges() {
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
            let caller = function("forward");
            let caller_position = checker
                .module
                .functions
                .iter()
                .position(|function| function.id == caller)
                .unwrap();
            let indices = super::super::build_sparse(checker).unwrap();
            let callees = &indices[&caller].callees;
            assert_eq!(callees.len(), 3);
            for callee in ["ordinary", "fallible", "stop"] {
                assert!(callees.contains(&function(callee)), "{callee}");
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
            let cfg = super::SparseCfg::new(checker, &snapshot, |_| false, false, &mut 0);
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

fn facts<'a>(checker: &'a BorrowChecker<'_>, function: FunctionId) -> &'a TypedCallAllocationFacts {
    let FunctionReturnAliasSummary::Known(facts) =
        &checker.public_call_summaries[&function].return_alias
    else {
        panic!("local HIR should publish a known summary");
    };
    facts
}

fn for_each_post(lane: &Lane, mut visit: impl FnMut(&Post)) {
    lane.visit_compatible(
        |_| Ok::<_, Infallible>(None),
        |_, product| {
            for post in product
                .components()
                .iter()
                .flat_map(|component| &component.alternatives)
            {
                visit(post);
            }
            Ok(())
        },
    )
    .unwrap();
}

fn entry(
    facts: &TypedCallAllocationFacts,
    parameter: usize,
    path: &[CallField],
) -> Option<ObjectRef> {
    facts
        .ports
        .iter()
        .position(|port| {
            matches!(port, EntryPort::Value { parameter: index, path: entry }
            if *index == parameter && entry.as_ref() == path)
        })
        .map(|port| ObjectRef::Entry(PortId(port as u32)))
}

fn allocation(source: &ValueSource) -> Option<ObjectRef> {
    match source {
        ValueSource::Allocation(object) => Some(object.clone()),
        ValueSource::Scalar(_) => None,
    }
}

fn cell_objects(post: &Post, cell: &CellPort) -> Option<Vec<ObjectRef>> {
    post.cells.iter().find_map(|(target, edit)| {
        if target != cell {
            return None;
        }
        let (CellPost::Replace(values) | CellPost::WeakJoin(values)) = edit;
        Some(
            values
                .iter()
                .flatten()
                .filter_map(|value| allocation(&value.source))
                .collect(),
        )
    })
}

fn result_objects(
    facts: &TypedCallAllocationFacts,
    post: &Post,
    slot: usize,
    path: &[CallField],
) -> Vec<ObjectRef> {
    let mut values: Vec<_> = post
        .results
        .iter()
        .filter_map(|(index, value)| {
            (*index == slot)
                .then(|| allocation(&value.source))
                .flatten()
        })
        .collect();
    for &selector in path {
        values = values
            .into_iter()
            .flat_map(|holder| {
                cell_objects(post, &CellPort::Edge(holder.clone(), selector)).unwrap_or_else(|| {
                    let ObjectRef::Entry(port) = holder else {
                        return Vec::new();
                    };
                    let EntryPort::Value { parameter, path } = &facts.ports[port.0 as usize] else {
                        return Vec::new();
                    };
                    let mut path = path.to_vec();
                    path.push(selector);
                    entry(facts, *parameter, &path).into_iter().collect()
                })
            })
            .collect();
    }
    values
}

fn has_result_entry(
    facts: &TypedCallAllocationFacts,
    slot: usize,
    path: &[CallField],
    parameter: usize,
    entry_path: &[CallField],
) -> bool {
    let Some(source) = entry(facts, parameter, entry_path) else {
        return false;
    };
    let mut found = false;
    for_each_post(&facts.success, |post| {
        found |= result_objects(facts, post, slot, path).contains(&source);
    });
    found
}

fn has_fresh_result(facts: &TypedCallAllocationFacts, slot: usize, path: &[CallField]) -> bool {
    let mut found = false;
    for_each_post(&facts.success, |post| {
        found |= result_objects(facts, post, slot, path)
            .iter()
            .any(|value| matches!(value, ObjectRef::Exact(_) | ObjectRef::Family(..)));
    });
    found
}

fn has_entry_cell_write(
    facts: &TypedCallAllocationFacts,
    target: (usize, &[CallField]),
    source: Option<(usize, &[CallField])>,
) -> bool {
    let Some((&selector, holder_path)) = target.1.split_last() else {
        return false;
    };
    let Some(holder) = entry(facts, target.0, holder_path) else {
        return false;
    };
    let cell = CellPort::Edge(holder, selector);
    let mut found = false;
    for_each_post(&facts.success, |post| {
        found |= cell_objects(post, &cell).is_some_and(|values| match source {
            Some((parameter, path)) => {
                entry(facts, parameter, path).is_some_and(|source| values.contains(&source))
            }
            None => values
                .iter()
                .any(|value| !matches!(value, ObjectRef::Entry(_))),
        });
    });
    found
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
            let facts_for = |name| facts(checker, function(name));
            let wrap = facts_for("wrap");
            assert!(has_fresh_result(wrap, 0, &[]), "wrap facts: {wrap:#?}");
            assert!(has_result_entry(facts_for("wrap"), 0, &items, 0, &[]));
            assert!(has_fresh_result(facts_for("wrap_copy"), 0, &items));
            assert!(has_result_entry(
                facts_for("unwrap_link"),
                0,
                &[],
                0,
                &items
            ));
            assert!(has_result_entry(facts_for("unwrap_temp"), 0, &[], 0, &[]));
            assert!(has_result_entry(facts_for("ordered"), 1, &[], 1, &[]));
            assert!(has_fresh_result(facts_for("ordered"), 2, &[]));
            assert!(has_fresh_result(facts_for("changed_field"), 0, &items));
            assert!(!has_result_entry(
                facts_for("changed_field"),
                0,
                &items,
                0,
                &[]
            ));
            assert!(has_result_entry(
                facts_for("shared_view"),
                0,
                &items,
                0,
                &[]
            ));
            assert_eq!(facts_for("recursive").success, Lane::Bottom);
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
                let left = [CallField::Field(0)];
                let right = [CallField::Field(1)];
                let selected = facts(checker, function("select"));
                assert!(
                    has_result_entry(selected, 0, &left, 1, &[]),
                    "missing left source: {selected:#?}"
                );
                assert!(
                    has_result_entry(selected, 0, &right, 2, &[]),
                    "missing right source: {selected:#?}"
                );
                let (a, b) = (
                    entry(selected, 1, &[]).unwrap(),
                    entry(selected, 2, &[]).unwrap(),
                );
                let mut together = false;
                for_each_post(&selected.success, |post| {
                    together |= result_objects(selected, post, 0, &left).contains(&a)
                        && result_objects(selected, post, 0, &right).contains(&b);
                });
                assert_eq!(together, shares, "selected facts: {selected:#?}");
                let forwarded = facts(checker, function("forward"));
                let mut shares_object = false;
                for_each_post(&forwarded.success, |post| {
                    let b = result_objects(forwarded, post, 0, &right);
                    shares_object |= result_objects(forwarded, post, 0, &left)
                        .iter()
                        .any(|a| !matches!(a, ObjectRef::Family(..)) && b.contains(a));
                });
                assert_eq!(shares_object, shares, "forwarded facts: {forwarded:#?}");
                assert!(has_fresh_result(forwarded, 0, &left));
                assert!(has_fresh_result(forwarded, 0, &right));
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
            let forwarded = facts(checker, function("forward"));
            assert!(has_result_entry(forwarded, 0, &[CallField::Field(0)], 0, a));
            assert!(has_result_entry(forwarded, 0, &[CallField::Field(1)], 0, b));
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
            let forwarded = facts(checker, function("forward"));
            let mut pairs = [false; 3];
            for_each_post(&forwarded.success, |post| {
                let [left, middle, right] = [0, 1, 2]
                    .map(|field| result_objects(forwarded, post, 0, &[CallField::Field(field)]));
                for (index, (a, b)) in [(&left, &middle), (&middle, &right), (&left, &right)]
                    .into_iter()
                    .enumerate()
                {
                    pairs[index] |= a
                        .iter()
                        .any(|a| !matches!(a, ObjectRef::Family(..)) && b.contains(a));
                }
            });
            assert_eq!(pairs, [true, true, false]);
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
            let transports = |facts: &TypedCallAllocationFacts, lane: &Lane| {
                let family = FamilyPort {
                    receiver: entry(facts, 0, &[]).unwrap(),
                    selector: CallField::Elem,
                };
                let child = entry(facts, 1, &[]).unwrap();
                let mut found = false;
                for_each_post(lane, |post| {
                    found |= post.families.iter().any(|(target, update)| {
                        let (FamilyPost::Extend(values) | FamilyPost::ReplaceAll(values)) = update;
                        *target == family
                            && values.iter().any(|(value, _)| {
                                value.source == ValueSource::Allocation(child.clone())
                            })
                    });
                });
                found
            };
            let failure = facts(checker, function("fail_after_write"));
            assert!(failure.success.can_return() && failure.error.can_return());
            assert!(!transports(failure, &failure.success));
            assert!(transports(failure, &failure.error));
            let success = facts(checker, function("write_on_success"));
            assert!(transports(success, &success.success));
            assert!(!transports(success, &success.error));
            let caught = facts(checker, function("caught"));
            assert!(transports(caught, &caught.success));
            assert_eq!(caught.error, Lane::Bottom);
        },
    );
}

#[test]
fn captured_option_payload_and_new_stored_subtree_keep_distinct_store_coordinates() {
    use crate::compiler_frontend::analysis::borrow_checker::state::{
        AddressSet, BorrowState, CapturedValue, CellId, Coverage, FieldKey, StoredValue,
        TypedStore, UNKNOWN_ORDER_KEY,
    };
    use crate::compiler_frontend::analysis::borrow_checker::transfer::typed_call::{
        EntryCapture, ExplicitMatchInput, match_typed_lane,
    };
    use crate::compiler_frontend::analysis::borrow_checker::{
        engine::apply_captured_call_batch,
        transfer::{CapturedCallBatch, CapturedDestination, EdgeTransferView, OperationView},
    };
    use crate::compiler_frontend::canonical_type_identity::{
        CanonicalBuiltinType, CanonicalTypeIdentity,
    };
    use crate::compiler_frontend::datatypes::ids::TypeId;
    use crate::compiler_frontend::public_call_summary::CallResultPath;
    use crate::compiler_frontend::public_call_summary::typed::{
        CellPort, CellPost, Component, ExportPort, Lane, ObjectRef, PathAccess, PortId, Post,
        Product, StorageKind, SupportId, ValueRef, ValueSource,
    };

    let mut state = BorrowState::new_uninitialized(6);
    let [
        option,
        original,
        previous_items,
        new_items,
        _result_slot,
        original_items,
    ] = std::array::from_fn(|index| {
        state.initialize_parameter(index);
        state.local_alloc_ids(index).0[0]
    });
    let replacement = state.call_site_id(
        5,
        false,
        0,
        false,
        crate::compiler_frontend::hir::ids::BlockId(0),
        &Default::default(),
    );
    let payload = CallField::Payload {
        variant: 1,
        field: 0,
    };
    let option_key = FieldKey::Payload {
        variant: 1,
        field: 0,
    };
    let items_key = FieldKey::Field(0);
    let stored = |id| StoredValue { id, writable: true };
    for (holder, key, value) in [
        (option, option_key, original),
        (original, items_key, original_items),
        (replacement, items_key, original_items),
    ] {
        let address = AddressSet::from_cells([CellId::Projected(holder, key)], Coverage::Complete)
            .with_singleton_proof();
        state.publish_store(
            &TypedStore::replace_cells(
                address,
                CapturedValue::new(vec![stored(value)], Vec::new()),
            ),
            UNKNOWN_ORDER_KEY,
            None,
        );
    }
    let mut old_payload = state.resolve_place_path(0, &[true; 6], [option_key]);
    state.load_place_content(&mut old_payload, &[true; 6], false);
    let captured_original = old_payload.content.unwrap().ids[0];
    assert_eq!(captured_original, original);

    let returned = ObjectRef::Exact(ExportPort::Result(CallResultPath {
        slot: 0,
        path: Vec::new(),
    }));
    let value = |source| ValueRef {
        source: ValueSource::Allocation(source),
        access: PathAccess {
            restriction: None,
            derived_from: None,
        },
        dependencies: Vec::new(),
    };
    let entry = |port| ObjectRef::Entry(PortId(port));
    let replace = |holder, field, source| {
        (
            CellPort::Edge(holder, field),
            CellPost::Replace(vec![Some(value(source))]),
        )
    };
    let post = Post {
        cells: vec![
            replace(entry(0), payload, returned.clone()),
            replace(entry(1), CallField::Field(0), entry(2)),
            replace(returned.clone(), CallField::Field(0), entry(3)),
        ],
        results: vec![(0, value(returned.clone()))],
        retained: vec![(
            returned.clone(),
            StorageKind::Allocation,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Range),
        )],
        ..Default::default()
    };
    let lane = Lane::Returning(
        Product::new(vec![Component {
            support: SupportId(0),
            alternatives: vec![post],
        }])
        .unwrap(),
    );
    let capture = |id| EntryCapture {
        ty: Some(TypeId(0)),
        writable: Some(true),
        value: CapturedValue::new(vec![stored(id)], Vec::new()),
        allocation_coverage: Some(Coverage::Complete),
        ..Default::default()
    };
    let mut matched = match_typed_lane(
        &ExplicitMatchInput {
            state: &state,
            entries: &[
                capture(option),
                capture(captured_original),
                capture(previous_items),
                capture(new_items),
            ],
            retained: &[(returned, capture(replacement))],
            uncertainty: &[],
        },
        &lane,
    )
    .unwrap();
    let post = matched
        .pop()
        .unwrap()
        .components
        .pop()
        .unwrap()
        .alternatives
        .pop()
        .unwrap();
    let operations = OperationView {
        local_ids: &[
            crate::compiler_frontend::hir::ids::LocalId(0),
            crate::compiler_frontend::hir::ids::LocalId(1),
            crate::compiler_frontend::hir::ids::LocalId(2),
            crate::compiler_frontend::hir::ids::LocalId(3),
            crate::compiler_frontend::hir::ids::LocalId(4),
            crate::compiler_frontend::hir::ids::LocalId(5),
        ],
        local_indices: &Default::default(),
        local_mutable: &[true; 6],
        local_has_in_place_mutation_surface: &[true; 6],
        block_successors: &Default::default(),
        local_first_write_order: &[0; 6],
        acyclic_entry_block: None,
        entry_roots: None,
    };
    apply_captured_call_batch(
        EdgeTransferView { operations },
        CapturedCallBatch {
            post,
            destinations: vec![CapturedDestination::Define(4)],
            member_reads: Default::default(),
            checkpoints: Vec::new(),
            detachment: None,
        },
        [],
        UNKNOWN_ORDER_KEY,
        &mut state,
    )
    .unwrap();
    assert_eq!(state.local_alloc_ids(4).0, [replacement]);
    let read = |local, path: &[FieldKey]| {
        let mut place = state.resolve_place_path(local, &[true; 6], path.iter().copied());
        state.load_place_content(&mut place, &[true; 6], false);
        place.content.unwrap().ids
    };
    assert_eq!(read(0, &[option_key, items_key]), [new_items]);
    assert_eq!(read(1, &[items_key]), [previous_items]);
}

#[test]
fn a_result_and_projected_store_share_one_retained_object() {
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
            for name in ["replace", "through_call"] {
                let facts = facts(checker, function(name));
                let store = CellPort::Edge(entry(facts, 0, &[]).unwrap(), field);
                let nested = entry(facts, 1, &[]).unwrap();
                let mut shared = false;
                for_each_post(&facts.success, |post| {
                    let stored = cell_objects(post, &store).unwrap_or_default();
                    shared |= result_objects(facts, post, 0, &[]).iter().any(|result| {
                        matches!(result, ObjectRef::Exact(_))
                            && stored.contains(result)
                            && cell_objects(post, &CellPort::Edge(result.clone(), field))
                                .is_some_and(|values| values.contains(&nested))
                    });
                });
                assert!(shared, "{name}");
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
            let items = [CallField::Field(0)];
            let scratch = facts(checker, function("unreturned_scratch"));
            assert!(has_fresh_result(scratch, 0, &items));
            assert!(!has_result_entry(scratch, 0, &items, 0, &[]));
            let retained = facts(checker, function("retained_holder"));
            assert!(has_result_entry(retained, 0, &items, 0, &[]));
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
    let items = [CallField::Field(0)];
    let payload = [CallField::Payload {
        variant: 1,
        field: 0,
    }];
    with_results(retained, |function, checker| {
        let facts = facts(checker, function("wrap_option"));
        assert!(has_result_entry(facts, 0, &items, 0, &payload));
        assert!(has_fresh_result(facts, 0, &items));
    });
    with_results(fresh, |function, checker| {
        let facts = facts(checker, function("fresh_wrap_option"));
        assert!(has_fresh_result(facts, 0, &items));
        assert!(!has_result_entry(facts, 0, &items, 0, &payload));
    });
}

#[test]
fn returned_option_payload_keeps_the_caller_argument_borrowed() {
    let fixture = ResultFixture::new(
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
    );
    fixture.with_results(|function, checker| {
        let unwrap_link = function("unwrap_link");
        assert!(has_result_entry(
            facts(checker, unwrap_link),
            0,
            &[],
            0,
            &[CallField::Payload {
                variant: 1,
                field: 0
            }],
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
        let report = fixture
            .checker()
            .run(None)
            .expect("the caller can read the returned payload after the call");
        assert_eq!(
            report
                .analysis
                .value_fact(argument)
                .unwrap()
                .optional_transfer,
            OptionalTransferStatus::Borrow
        );
    });
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
            let facts = facts(checker, function("through"));
            let items: (usize, &[CallField]) = (0, &[CallField::Field(0)]);
            assert!(has_entry_cell_write(facts, items, Some((2, &[]))));
            assert!(!has_entry_cell_write(facts, items, Some((1, &[]))));
            let payload = CallField::Payload {
                variant: 1,
                field: 0,
            };
            assert!(has_result_entry(facts, 0, &[payload], 0, &[]));
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
        FunctionReturnAliasSummary::Known(facts) => facts,
        other => panic!("relay should publish a known summary, got {other:?}"),
    };
    assert!(has_entry_cell_write(
        relay,
        (0, &[CallField::Field(0)]),
        Some((1, &[]))
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
                    lane.can_return(),
                    "{mutation}: the propagated `none` exit still succeeds"
                );
                for_each_post(lane, |post| {
                    let caller_owned = |object: &ObjectRef| {
                        matches!(
                            object,
                            ObjectRef::Entry(_)
                                | ObjectRef::Exact(ExportPort::Binding(_) | ExportPort::Cell(_))
                        )
                    };
                    let caller_cell = |cell: &CellPort| match cell {
                        CellPort::Terminal(_) => true,
                        CellPort::Slot(object) | CellPort::Edge(object, _) => caller_owned(object),
                    };
                    assert!(
                        !post.bindings.iter().any(|(object, _)| caller_owned(object))
                            && !post.cells.iter().any(|(cell, _)| caller_cell(cell))
                            && !post
                                .families
                                .iter()
                                .any(|(port, _)| caller_owned(&port.receiver)),
                        "{mutation}: the propagated exit must not change caller storage: {post:?}"
                    );
                    assert!(post.results.iter().any(|(slot, value)| {
                        *slot == 0 && post.cells.iter().any(|(cell, edit)| {
                            matches!((&value.source, cell),
                                (ValueSource::Allocation(result),
                                 CellPort::Edge(holder, CallField::Payload { variant: 1, field: 0 }))
                                    if result == holder)
                                && matches!(edit, CellPost::Replace(values) if matches!(values.as_slice(), [None]))
                        })
                    }), "{mutation}: the propagated result must be known absent: {post:?}");
                });
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
            for name in ["stash", "stash_after_self_update"] {
                let facts = facts(checker, function(name));
                assert!(has_result_entry(facts, 0, &[], 1, &[]), "{name}");
                assert!(
                    !has_result_entry(facts, 0, &[], 0, &[CallField::Field(0)]),
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
            let facts = facts(checker, function("stash"));
            assert!(has_entry_cell_write(
                facts,
                (1, &[CallField::Field(0)]),
                None
            ));
            assert!(has_entry_cell_write(
                facts,
                (2, &[CallField::Field(1)]),
                None
            ));
            for (parameter, ordinal) in [(1, 1), (2, 0)] {
                let pad = CellPort::Edge(
                    entry(facts, parameter, &[]).unwrap(),
                    CallField::Field(ordinal),
                );
                for_each_post(&facts.success, |post| {
                    assert!(post.cells.iter().all(|(target, _)| *target != pad));
                });
            }
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
    assert!(has_result_entry(facts, 0, &[], 3, &[]));
    assert!(has_result_entry(facts, 0, &[], 2, &[CallField::Field(0)]));
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
            assert!(has_result_entry(
                facts(checker, function("stash")),
                0,
                &[],
                2,
                &[]
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
            let facts = facts(checker, function("stash"));
            assert!(has_entry_cell_write(
                facts,
                (2, &[CallField::Field(0)]),
                Some((3, &[]))
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
            // The catch exits return `incoming`, so a fresh object can only come from the normal exit.
            assert!(has_fresh_result(facts(checker, function("probe")), 0, &[]));
        },
    );
}

#[test]
fn pending_call_emits_no_continuation_until_contract_arrives() {
    let fixture = ResultFixture::new(
        r#"
immediate_forward |value I32| -> I32:
    seen = immediate(value)
    return seen
;

invoke_forward |ok Bool, value I32| -> I32, Error!:
    return fallible(ok, value)!
;

never_forward |dest ~I32, value ~I32| -> !:
    stop(~dest, ~value)
;

immediate |value I32| -> I32:
    return value
;

fallible |ok Bool, value I32| -> I32, Error!:
    if ok:
        return value
    ;
    return! Error("failure")
;

stop |dest ~I32, value ~I32| -> !:
    ~dest = value
    loop true:
        io.line([:[dest]])
    ;
;
"#,
    );
    let mut checker = fixture.checker();
    checker.public_call_summaries =
        BorrowChecker::seed_public_call_summaries(checker.module, &checker.diagnostics).unwrap();
    for (forward, leaf, returns, fails, retained_value) in [
        ("immediate_forward", "immediate", true, false, 0),
        ("invoke_forward", "fallible", true, true, 1),
        ("never_forward", "stop", false, false, 1),
    ] {
        let forward = named_function(&checker, forward);
        let leaf = named_function(&checker, leaf);
        let indices = super::super::build_sparse(&checker).unwrap();
        let classify = |checker: &BorrowChecker<'_>, function: &HirFunction| {
            let FunctionReturnAliasSummary::Known(facts) = checker
                .classify_function_return_alias(function, &indices[&function.id])
                .unwrap()
            else {
                panic!("extraction must produce a provisional transfer summary")
            };
            facts
        };
        let before = classify(&checker, forward);
        assert_eq!(before.success, Lane::Bottom);
        assert_eq!(before.error, Lane::Bottom);
        assert!(before.may_retain.is_empty());
        let leaf_id = leaf.id;
        let leaf = classify(&checker, leaf);
        checker
            .public_call_summaries
            .get_mut(&leaf_id)
            .unwrap()
            .return_alias = FunctionReturnAliasSummary::Known(leaf);
        let after = classify(&checker, forward);
        assert_eq!(after.success.can_return(), returns);
        assert_eq!(after.error.can_return(), fails);
        assert_eq!(after.success.result_extent(), usize::from(returns));
        assert_eq!(after.error.result_extent(), usize::from(fails));
        if returns {
            use crate::compiler_frontend::public_call_summary::typed::ScalarSource;
            // Returning a scalar retains its source on the selected lane, not on execution.
            assert!(after.may_retain.is_empty());
            let mut retained = false;
            for_each_post(&after.success, |post| {
                retained |= post.results.iter().any(|(slot, value)| {
                    let ValueSource::Scalar(ScalarSource::EntryValue(port)) = value.source else {
                        return false;
                    };
                    *slot == 0 && matches!(&after.ports[port.0 as usize],
                        EntryPort::Value { parameter, path } if *parameter == retained_value && path.is_empty())
                });
            });
            assert!(retained);
        } else {
            assert_eq!(after.may_retain, [retained_value]);
        }
    }
}

#[test]
fn replacing_an_empty_caller_cell_keeps_member_success_reachable() {
    let fixture = ResultFixture::new(
        r#"
Box = |items {I32}|

seed |boxes ~{Box}|:
    ~boxes = {Box({0})}
;

take || -> I32, Error!:
    boxes ~{Box} = {}
    seed(~boxes)
    removed ~= ~boxes.remove(0) catch |err|:
        return! err
    ;
    ~removed.items = {1}
    return removed.items.length()
;

lookup || -> I32, Error!:
    boxes ~{Box} = {}
    seed(~boxes)
    found = boxes.get(0)!
    return found.items.length()
;
"#,
    );
    fixture.with_results(|function, checker| {
        let seed = facts(checker, function("seed"));
        let mut populated_replacement = false;
        for_each_post(&seed.success, |post| {
            for (port, edit) in &post.families {
                if port.selector == CallField::Elem
                    && matches!(edit, FamilyPost::ReplaceAll(members) if !members.is_empty())
                    && post.cells.iter().any(|(cell, edit)| {
                        matches!(cell, CellPort::Terminal(_))
                            && matches!(edit, CellPost::Replace(values) if values.iter().flatten().any(|value| {
                                value.source == ValueSource::Allocation(port.receiver.clone())
                            }))
                    })
                {
                    populated_replacement = true;
                }
            }
        });
        assert!(populated_replacement, "the replacement installs a populated graph on the caller's cell");
        for callee in ["take", "lookup"] {
            assert!(facts(checker, function(callee)).success.can_return(), "{callee}");
        }
    });
}

#[test]
fn terminating_call_uses_executed_retention_without_a_lane() {
    let leaf = r#"
link_forever |dest ~I32, value ~I32| -> !:
    ~dest = value
    loop true:
        io.line([:[dest]])
    ;
;
"#;
    let forward = r#"
forward_forever |dest ~I32, value ~I32| -> !:
    link_forever(~dest, ~value)
;
"#;
    let outer = r#"
outer || -> !:
    dest ~= 1
    value ~= 2
    forward_forever(~dest, ~value)
;
"#;
    for (order, source) in [
        [outer, forward, leaf].concat(),
        [leaf, forward, outer].concat(),
        [outer, forward, &leaf.replace("~dest = value", "~dest = 0")].concat(),
    ]
    .into_iter()
    .enumerate()
    {
        let retaining = order < 2;
        let fixture = ResultFixture::new(&source);
        fixture.with_results(|function, checker| {
            for callee in ["link_forever", "forward_forever", "outer"] {
                let facts = facts(checker, function(callee));
                assert_eq!(facts.success, Lane::Bottom);
                assert_eq!(facts.error, Lane::Bottom);
                if callee != "outer" {
                    assert_eq!(
                        facts.may_retain,
                        if retaining { &[1][..] } else { &[][..] },
                        "{callee}"
                    );
                }
            }
            let outer = named_function(checker, "outer");
            let blocks = checker.collect_reachable_blocks(outer).unwrap();
            let args = blocks
                .iter()
                .find_map(|&id| {
                    let block = checker.block_by_id_or_error(id, outer.id).unwrap();
                    match &block.terminator {
                        super::HirTerminator::NeverCall {
                            target: CallTarget::Local(target),
                            args,
                            ..
                        } if *target == function("forward_forever") => Some(*args),
                        _ => None,
                    }
                })
                .expect("the outer call is terminal, with no receiving locals");
            let report = fixture
                .checker()
                .run(None)
                .expect("independent arguments are legal on a terminating call");
            let value = checker.module.expressions.values(args)[1];
            assert_eq!(
                report.analysis.value_fact(value).unwrap().optional_transfer,
                if retaining {
                    OptionalTransferStatus::Borrow
                } else {
                    OptionalTransferStatus::Transfer
                },
            );
            assert_eq!(
                report.analysis.public_call_summaries[&function("forward_forever")].parameters[0]
                    .mutation,
                crate::compiler_frontend::public_call_summary::PublicCallMutationEffect::Writes,
            );
        });
    }
}

//! Tests the folded publication transaction and its work and memory boundaries.

use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstValueMetadata;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;

fn metadata() -> ConstValueMetadata {
    ConstValueMetadata::resolved(builtin_type_ids::BOOL, None, ValueMode::ImmutableOwned)
}

fn assert_expansion(
    candidate: &ConstValueCandidate<'_>,
    roots: &[ConstValueId],
    steps: usize,
    count: usize,
) {
    for allowance in [steps, steps - 1] {
        let meter = ConstValueStore::default().execution_meter();
        meter.borrow_mut().limits.max_attempt_steps = allowance;
        let mut budget = ExecutionBudget::new(meter.clone());
        let result = expanded_occurrences(candidate, roots, &mut budget);
        if allowance == steps {
            assert_eq!(result.expect("exact step allowance"), count);
        } else {
            assert!(matches!(
                result,
                Err(ConstantFoldError::Budget(exhaustion))
                    if exhaustion.unit == ConstBudgetUnit::Steps
            ));
        }
        assert_eq!(budget.steps.get(), steps);
        assert_eq!(budget.retained_memory_bytes, 0);
        assert_eq!(meter.borrow().retained_memory_bytes, 0);
    }
}

#[test]
fn expanded_occurrences_counts_an_ordinary_tree() -> Result<(), ConstantFoldError> {
    ConstValueStore::default().stage_values(|candidate| {
        let start = candidate.insert_value(metadata(), ConstValuePayload::Bool(true))?;
        let end = candidate.insert_value(metadata(), ConstValuePayload::Bool(false))?;
        let range = candidate.insert_value(metadata(), ConstValuePayload::Range { start, end })?;
        let side = candidate.insert_value(metadata(), ConstValuePayload::Bool(true))?;
        let root =
            candidate.insert_value(metadata(), ConstValuePayload::Collection(vec![range, side]))?;
        // One root reference, four child references and five distinct definitions.
        assert_expansion(candidate, &[root], 10, 5);
        Ok::<_, ConstantFoldError>(Some(root))
    })?;
    Ok(())
}

#[test]
fn expanded_occurrences_counts_shared_edges_and_roots() -> Result<(), ConstantFoldError> {
    ConstValueStore::default().stage_values(|candidate| {
        let leaf = candidate.insert_value(metadata(), ConstValuePayload::Bool(true))?;
        let child =
            candidate.insert_value(metadata(), ConstValuePayload::Collection(vec![leaf]))?;
        candidate.mark_shared_allocation(child)?;
        let first = candidate.insert_shared_occurrence(metadata(), child)?;
        let second = candidate.insert_shared_occurrence(metadata(), child)?;
        let left = candidate.insert_value(
            metadata(),
            ConstValuePayload::Collection(vec![child, first]),
        )?;
        let right = candidate.insert_value(
            metadata(),
            ConstValuePayload::Collection(vec![first, second]),
        )?;
        let root =
            candidate.insert_value(metadata(), ConstValuePayload::Collection(vec![left, right]))?;
        let roots = vec![root, first, root];
        // Three roots, seven child references and five definitions. The two diamond
        // roots each expand to eleven occurrences; the middle root expands to two.
        assert_expansion(candidate, &roots, 15, 24);
        Ok::<_, ConstantFoldError>(Some(roots))
    })?;
    Ok(())
}

#[test]
fn const_freeze_charges_definition_occurrence_and_owned_edge_once() -> Result<(), ConstantFoldError>
{
    use super::super::budget::MAX_RETAINED_VALUES;

    for slots in [3, 2] {
        let mut store = ConstValueStore::default();
        let leaf =
            store.insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)?;
        let mut insert_shape = || {
            store.insert_resolved(
                builtin_type_ids::BOOL,
                ConstValuePayload::OptionSome(leaf),
                None,
            )
        };
        let shape = insert_shape()?;
        let alternate = insert_shape()?;
        let mut budget = ExecutionBudget::new(store.execution_meter());
        let mut storage = LiveStorage::default();
        let live = storage.allocate(
            shape,
            [LiveValue::leaf(leaf, Permission::Writable)].into_iter(),
            &mut budget,
        )?;
        let mut occurrence = live;
        occurrence.shape = alternate;
        let before_bytes = budget.retained_memory_bytes;
        budget.retained_values = MAX_RETAINED_VALUES - slots;
        let result = budget.with_scratch(|execution, scratch| {
            let mut publication = PublicationBudget { execution, scratch };
            store.stage_values_with_budget(&mut publication, |candidate, publication| {
                let mut memo = FreezeMemo::default();
                let roots = (|| {
                    let definition =
                        freeze_staged(candidate, &storage, publication, live, 0, &mut memo)?;
                    let occurrence =
                        freeze_staged(candidate, &storage, publication, occurrence, 0, &mut memo)?;
                    Ok::<_, ConstantFoldError>(Some(vec![definition, occurrence]))
                })();
                let memo_bytes = memo.scratch_bytes;
                drop(memo);
                publication.execution.release_memory_bytes(memo_bytes);
                roots
            })
        });
        if slots == 3 {
            let roots = result.expect("exact slot allowance").expect("two roots");
            assert_eq!(store.shared_allocation(roots[1]), Some(roots[0]));
            assert_eq!(budget.retained_values, MAX_RETAINED_VALUES);
            assert_eq!(
                budget.retained_memory_bytes - before_bytes,
                2 * std::mem::size_of::<ConstValue>()
            );
        } else {
            assert!(matches!(
                result,
                Err(ConstantFoldError::Budget(exhaustion))
                    if exhaustion.unit == ConstBudgetUnit::RetainedValues
            ));
            assert_eq!(store.value_count(), 3);
        }
    }
    Ok(())
}

fn store_with_prefix(count: usize) -> (ConstValueStore, Vec<ConstValueId>) {
    let mut store = ConstValueStore::default();
    let roots = store
        .stage_values(|candidate| {
            (0..count)
                .map(|_| candidate.insert_value(metadata(), ConstValuePayload::Bool(true)))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        })
        .expect("prefix insertion")
        .expect("prefix roots");
    (store, roots)
}

#[test]
fn stage_values_budget_checks_peak_with_and_without_destination_growth() {
    let row = std::mem::size_of::<ConstValue>();
    let id = std::mem::size_of::<ConstValueId>();
    // Both prefixes start with capacity four. Charge four staged row slots, one
    // logical result row and one remap entry. The destination adds two unused
    // slots without growth, or three unused and four overlapping old slots with it.
    for (prefix, extra) in [(1, 2), (4, 7)] {
        let peak = (4 + 1 + extra) * row + id;
        for allowance in [peak, peak - 1] {
            let (mut store, roots) = store_with_prefix(prefix);
            let meter = store.execution_meter();
            meter.borrow_mut().limits.max_attempt_memory_bytes = allowance;
            let mut budget = ExecutionBudget::new(meter.clone());
            let result = budget.with_scratch(|execution, scratch| {
                let mut budget = PublicationBudget { execution, scratch };
                store.stage_values_with_budget(&mut budget, |candidate, budget| {
                    budget.execution.retain_memory_bytes(row, None)?;
                    candidate.reserve_staged_row(budget)?;
                    let staged =
                        candidate.insert_value(metadata(), ConstValuePayload::Bool(false))?;
                    Ok(Some(staged))
                })
            });
            if allowance == peak {
                let committed = result.expect("exact limit").expect("promoted row");
                assert!(matches!(
                    store.payload(committed),
                    Some(ConstValuePayload::Bool(false))
                ));
                assert_eq!(store.value_count(), prefix + 1);
            } else {
                assert!(matches!(
                    result,
                    Err(ConstantFoldError::Budget(exhaustion))
                        if exhaustion.category == ConstBudgetCategory::Memory
                ));
                assert_eq!(store.value_count(), prefix);
            }
            for root in roots {
                assert!(matches!(
                    store.payload(root),
                    Some(ConstValuePayload::Bool(true))
                ));
            }
            assert_eq!(budget.retained_memory_bytes, row);
            assert_eq!(meter.borrow().retained_memory_bytes, row);
        }
    }
}

#[test]
fn stage_values_budget_failure_in_later_slot_drops_staged_capacity() {
    let row = std::mem::size_of::<ConstValue>();
    let (mut store, existing) = store_with_prefix(1);
    let meter = store.execution_meter();
    // Four staged rows fit; growing to eight while the original four remain live does not.
    meter.borrow_mut().limits.max_attempt_memory_bytes = 12 * row - 1;
    let mut budget = ExecutionBudget::new(meter.clone());
    let result = budget.with_scratch(|execution, scratch| {
        let mut budget = PublicationBudget { execution, scratch };
        store.stage_values_with_budget(&mut budget, |candidate, budget| {
            for _ in 0..5 {
                candidate.reserve_staged_row(budget)?;
                candidate.insert_value(metadata(), ConstValuePayload::Bool(false))?;
            }
            Ok(Some(existing[0]))
        })
    });
    assert!(matches!(
        result,
        Err(ConstantFoldError::Budget(exhaustion))
            if exhaustion.category == ConstBudgetCategory::Memory
    ));
    assert_eq!(store.value_count(), 1);
    assert!(matches!(
        store.payload(existing[0]),
        Some(ConstValuePayload::Bool(true))
    ));
    assert_eq!(budget.retained_memory_bytes, 0);
    assert_eq!(meter.borrow().retained_memory_bytes, 0);
}

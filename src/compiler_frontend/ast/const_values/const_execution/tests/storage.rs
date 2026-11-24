//! Access-generation and commit invariants unreachable through ordinary source.

use super::*;
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;

fn element() -> LiveValue {
    let shape = ConstValueStore::default()
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("boolean shape");
    LiveValue::leaf(shape, Permission::Writable)
}

#[test]
fn refused_graph_mutation_preserves_contents_and_access() {
    let constants = ConstValueStore::default();
    let meter = constants.execution_meter();
    let mut budget = ExecutionBudget::new(meter.clone());
    let mut storage = LiveStorage::default();
    let element = element();
    let child = storage
        .allocate(element.shape, [element].into_iter(), &mut budget)
        .expect("child allocation");
    let child_id = child.allocation.expect("child identity");
    let parent = storage
        .allocate(element.shape, [child].into_iter(), &mut budget)
        .expect("parent allocation");
    let read = storage
        .share(parent, AccessSite::default(), &mut budget)
        .expect("immutable view of parent");

    let limit = meter.borrow().limits.max_attempt_steps;
    meter.borrow_mut().limits.max_attempt_steps = budget.steps.get();
    let result = storage.push(child_id, element, AccessSite::default(), &mut budget);
    assert!(matches!(result, Err(ConstantFoldError::Budget(_))));
    meter.borrow_mut().limits.max_attempt_steps = limit;
    assert_eq!(storage.children(child_id).len(), 1);
    assert!(storage.validate_value(read, &mut budget).is_ok());

    assert!(matches!(
        storage.push(child_id, element, AccessSite::default(), &mut budget),
        Ok(true)
    ));
    assert!(matches!(
        storage.validate_value(read, &mut budget),
        Err(StorageError::InvalidAccess(_))
    ));
}

#[test]
fn exhausted_generation_cannot_wrap_or_commit() {
    let mut budget = ExecutionBudget::new(ConstValueStore::default().execution_meter());
    let mut storage = LiveStorage {
        epoch: u32::MAX,
        ..LiveStorage::default()
    };
    let element = element();
    let collection = storage
        .allocate(element.shape, [element].into_iter(), &mut budget)
        .expect("collection allocation");
    let allocation = collection.allocation.expect("collection identity");
    let read = storage
        .share(collection, AccessSite::default(), &mut budget)
        .expect("immutable view at the last generation");

    let result = storage.remove(allocation, 0, AccessSite::default(), &mut budget);
    assert!(matches!(result, Err(ConstantFoldError::Infrastructure(_))));
    assert_eq!(storage.epoch, u32::MAX);
    assert_eq!(storage.children(allocation).len(), 1);
    assert!(storage.validate_value(read, &mut budget).is_ok());
}

#[test]
fn removing_one_of_two_edges_does_not_detach_the_other_edge() {
    let mut budget = ExecutionBudget::new(ConstValueStore::default().execution_meter());
    let mut storage = LiveStorage::default();
    let element = element();
    let child = storage
        .allocate(element.shape, [element].into_iter(), &mut budget)
        .expect("child allocation");
    let child_id = child.allocation.expect("child identity");
    let parent = storage
        .allocate(element.shape, [child, child].into_iter(), &mut budget)
        .expect("shared edges");
    let parent_id = parent.allocation.expect("parent identity");
    storage
        .remove(parent_id, 0, AccessSite::default(), &mut budget)
        .expect("remove one edge");
    let read = storage
        .share(parent, AccessSite::default(), &mut budget)
        .expect("immutable view after the removal");

    storage
        .push(child_id, element, AccessSite::default(), &mut budget)
        .expect("grow the child retained by the remaining edge");
    assert!(matches!(
        storage.validate_value(read, &mut budget),
        Err(StorageError::InvalidAccess(_))
    ));
}

#[test]
fn repeated_retained_projection_is_checked_once_per_observation() {
    const COUNT: usize = 64;
    let meter = ConstValueStore::default().execution_meter();
    let mut budget = ExecutionBudget::new(meter.clone());
    let mut storage = LiveStorage::default();
    let site = AccessSite::default();
    let scalar = element();
    let other = storage
        .bind_slot(scalar, Permission::Writable, site, &mut budget)
        .expect("unrelated scalar");
    let mut retained = storage
        .share(scalar, site, &mut budget)
        .expect("read-only element");
    for _ in 0..COUNT {
        let owner = storage
            .allocate(scalar.shape, [retained].into_iter(), &mut budget)
            .expect("one retained edge");
        let place = storage
            .project(owner, 0)
            .expect("projection")
            .expect("edge");
        retained = match storage.read(place, site, &mut budget) {
            Ok(value) => value,
            Err(_) => panic!("no write has occurred"),
        };
    }
    let owner = storage
        .allocate(
            scalar.shape,
            std::iter::repeat_n(retained, COUNT),
            &mut budget,
        )
        .expect("many edges sharing their origin");
    let observed = storage
        .share(owner, site, &mut budget)
        .expect("shared owner");
    assert!(matches!(
        storage.replace(other.place(), scalar, site, &mut budget),
        Ok(true)
    ));

    // Plenty for the source edges and their one shared ancestry, not for
    // re-walking every origin for every incoming edge.
    budget.steps.set(0);
    meter.borrow_mut().limits.max_attempt_steps = 4 * COUNT;
    assert!(storage.validate_value(observed, &mut budget).is_ok());
}

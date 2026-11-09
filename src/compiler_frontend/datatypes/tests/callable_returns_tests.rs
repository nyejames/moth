//! Unit tests for `datatypes::callable_returns`.

use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::ids::TypeId;

fn shape(success: impl IntoIterator<Item = TypeId>, error: Option<TypeId>) -> CallableReturnShape {
    CallableReturnShape::from_slots(success, error)
}

#[test]
fn from_slots_normalises_zero_one_and_multiple() {
    let zero = shape([], None);
    assert_eq!(zero.success().len(), 0);
    assert_eq!(zero.single_success_slot(), None);
    assert_eq!(zero, CallableReturnShape::default());

    let one = shape([TypeId(3)], None);
    assert_eq!(one.success(), &[TypeId(3)]);
    assert_eq!(one.single_success_slot(), Some(TypeId(3)));

    let multiple = shape([TypeId(3), TypeId(4), TypeId(5)], Some(TypeId(9)));
    assert_eq!(multiple.success(), &[TypeId(3), TypeId(4), TypeId(5)]);
    assert_eq!(multiple.single_success_slot(), None);
    assert_eq!(multiple.error(), Some(TypeId(9)));
    assert_eq!(
        multiple,
        CallableReturnShape::from_slots(vec![TypeId(3), TypeId(4), TypeId(5)], Some(TypeId(9)))
    );
}

#[test]
fn success_slice_view_matches_normalised_payload() {
    assert_eq!(shape([], None).success(), &[]);
    assert_eq!(shape([TypeId(7)], None).success(), &[TypeId(7)]);
    assert_eq!(
        shape([TypeId(7), TypeId(8)], None).success(),
        &[TypeId(7), TypeId(8)]
    );
}

#[test]
fn default_is_an_empty_infallible_contract() {
    let default = CallableReturnShape::default();
    assert_eq!(default, shape([], None));
    assert_eq!(default.success(), &[]);
    assert_eq!(default.error(), None);
    assert!(!default.is_fallible());
}

#[test]
fn single_success_slot_only_answers_exactly_one() {
    assert_eq!(shape([], None).single_success_slot(), None);
    assert_eq!(
        shape([TypeId(1)], None).single_success_slot(),
        Some(TypeId(1))
    );
    assert_eq!(
        shape([TypeId(1), TypeId(2)], None).single_success_slot(),
        None
    );
}

#[test]
fn error_is_an_independent_channel() {
    let mut contract = shape([TypeId(1)], None);
    assert_ne!(contract, shape([TypeId(1)], Some(TypeId(2))));

    contract.set_error(Some(TypeId(2)));
    assert_eq!(contract.error(), Some(TypeId(2)));
    assert!(contract.is_fallible());
    assert_eq!(contract.success(), &[TypeId(1)]);

    contract.set_error(None);
    assert_eq!(contract.error(), None);
    assert!(!contract.is_fallible());
}

#[test]
fn map_type_ids_rewrites_success_and_error() {
    let contract = shape([TypeId(1), TypeId(2)], Some(TypeId(3)));
    let mapped = contract.map_type_ids(|type_id| TypeId(type_id.0 + 10));

    assert_eq!(mapped.success(), &[TypeId(11), TypeId(12)]);
    assert_eq!(mapped.error(), Some(TypeId(13)));
    assert_eq!(mapped, shape([TypeId(11), TypeId(12)], Some(TypeId(13))));
}

#[test]
fn map_type_ids_preserves_the_compact_variants() {
    assert_eq!(
        shape([], None).map_type_ids(|type_id| TypeId(type_id.0 + 1)),
        shape([], None)
    );
    assert_eq!(
        shape([TypeId(4)], None).map_type_ids(|type_id| TypeId(type_id.0 + 1)),
        shape([TypeId(5)], None)
    );
}

#[test]
fn equal_contracts_hash_and_compare_equal() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let first = shape([TypeId(1), TypeId(2)], Some(TypeId(3)));
    let second = shape([TypeId(1), TypeId(2)], Some(TypeId(3)));
    assert_eq!(first, second);

    let hash_of = |contract: &CallableReturnShape| {
        let mut hasher = DefaultHasher::new();
        contract.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash_of(&first), hash_of(&second));
}

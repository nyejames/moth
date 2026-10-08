//! Borrow metadata traversal regression tests.
//!
//! WHAT: checks that assertion-message expressions contribute their local loads to terminator
//!       metadata.
//! WHY: the message is an ordinary shared terminal use and must not be skipped as legacy text.

use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::{LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use rustc_hash::FxHashSet;

#[test]
fn assertion_failure_message_contributes_loaded_local_metadata() {
    let mut expressions = HirExpressionStore::default();
    let message = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(7))),
        builtin_type_ids::STRING,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let terminator = HirTerminator::AssertFailure {
        message,
        message_evaluation: HirAssertionMessageEvaluation::Runtime,
    };
    let mut loaded_locals = FxHashSet::default();

    super::collect_terminator_loaded_locals(&expressions, &terminator, &mut |local| {
        loaded_locals.insert(local);
    });

    assert_eq!(loaded_locals, FxHashSet::from_iter([LocalId(7)]));
}

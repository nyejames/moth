//! Borrow metadata traversal regression tests.
//!
//! WHAT: checks that assertion-message expressions contribute their local loads to terminator
//!       metadata.
//! WHY: the message is an ordinary shared terminal use and must not be skipped as legacy text.

use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use rustc_hash::FxHashSet;

#[test]
fn assertion_failure_message_contributes_loaded_local_metadata() {
    let message = HirExpression {
        id: HirValueId(41),
        kind: HirExpressionKind::Load(HirPlace::Local(LocalId(7))),
        ty: builtin_type_ids::STRING,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
    };
    let terminator = HirTerminator::AssertFailure {
        message,
        message_evaluation: HirAssertionMessageEvaluation::Runtime,
    };
    let mut loaded_locals = FxHashSet::default();

    super::collect_terminator_loaded_locals(&terminator, &mut |local| {
        loaded_locals.insert(local);
    });

    assert_eq!(loaded_locals, FxHashSet::from_iter([LocalId(7)]));
}

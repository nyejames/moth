//! Receiver-method diagnostic naming tests.
//!
//! WHAT: validates that receiver diagnostics use the authored receiver name rather than a path
//!       table identity.
//! WHY: receiver labels cross from internal receiver keys into user-facing diagnostics, where a
//!      leaked `PathId` makes the message meaningless.

use super::receiver_methods::receiver_kind_label;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn receiver_label_uses_the_authored_type_name_not_a_path_id() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let counter_path = path_fork
        .try_intern_portable_path("counter/Counter", &mut string_table)
        .expect("the test path fits the empty fork");

    let label_id = receiver_kind_label(
        &ReceiverKey::Struct(counter_path),
        &path_fork,
        &mut string_table,
    );
    let label = string_table.resolve(label_id);

    assert_eq!(label, "Counter");
    assert!(!label.contains("PathId"));
}

//! Focused unit tests for constant dependency invariant behaviour.
//!
//! WHAT: verifies that missing constant position metadata produces an internal compiler error,
//!       not a user-facing source diagnostic.
//! WHY: the position record is built in the same inventory pass as constant classification, so
//!      a missing record is a compiler invariant violation.

use super::*;
use std::path::PathBuf;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

#[test]
fn missing_constant_position_produces_infrastructure_error() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let constant_path = path_fork
        .try_intern_filesystem_path(&PathBuf::from("src/missing.moth"), &mut string_table)
        .expect("test path should be UTF-8");
    let error = missing_constant_position_error(&constant_path, &path_fork, &string_table);

    assert!(
        error.msg.contains("Missing constant position metadata"),
        "missing constant position must be an infrastructure error, not a user-facing diagnostic: {error:?}"
    );
}

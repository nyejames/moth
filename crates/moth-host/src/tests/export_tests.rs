//! Tests for the builder's explicit host-callable export selection.

use crate::{ExportSelectionError, HostExports};

fn selected_names(selection: &crate::HostExportSelection) -> Vec<(&str, &str)> {
    selection
        .exports()
        .iter()
        .map(|export| (export.facade_name(), export.export_name()))
        .collect()
}

#[test]
fn a_valid_selection_is_ordered_by_export_name() {
    let selection = HostExports::new()
        .export("update", "game_update")
        .export("start", "game_start")
        .finish()
        .expect("valid distinct export names should be accepted");

    assert_eq!(
        selected_names(&selection),
        [("start", "game_start"), ("update", "game_update")]
    );
}

#[test]
fn an_empty_export_name_is_rejected() {
    let error = HostExports::new()
        .export("update", "")
        .finish()
        .expect_err("an empty Wasm export name should be rejected");

    assert_eq!(
        error,
        ExportSelectionError::EmptyExportName {
            facade_name: "update".to_owned(),
        }
    );
}

#[test]
fn reserved_export_names_are_rejected() {
    for export_name in ["moth_update", "__moth_update", "memory"] {
        let error = HostExports::new()
            .export("update", export_name)
            .finish()
            .expect_err("a reserved Wasm export name should be rejected");

        assert_eq!(
            error,
            ExportSelectionError::ReservedExportName {
                facade_name: "update".to_owned(),
                export_name: export_name.to_owned(),
            }
        );
    }

    // The reserved names are fixed spellings, so a case-only variant does not collide with one.
    let selection = HostExports::new()
        .export("update", "Moth_x")
        .export("update", "Memory")
        .finish()
        .expect("reserved-name checks are case sensitive");

    assert_eq!(
        selected_names(&selection),
        [("update", "Memory"), ("update", "Moth_x")]
    );
}

#[test]
fn a_duplicate_export_name_is_rejected() {
    let error = HostExports::new()
        .export("update", "game_update")
        .export("tick", "game_update")
        .finish()
        .expect_err("a repeated Wasm export name should be rejected");

    assert_eq!(
        error,
        ExportSelectionError::DuplicateExportName {
            export_name: "game_update".to_owned(),
        }
    );
}

#[test]
fn one_facade_function_may_be_selected_under_two_export_names() {
    let selection = HostExports::new()
        .export("update", "game_update")
        .export("update", "game_tick")
        .finish()
        .expect("the native host ABI only fixes uniqueness of the Wasm export name");

    assert_eq!(
        selected_names(&selection),
        [("update", "game_tick"), ("update", "game_update")]
    );
}

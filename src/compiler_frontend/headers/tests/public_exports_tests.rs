//! Public-export collector regressions.
//!
//! WHAT: verifies that duplicate public exports retain the first authored owner across the
//! source-package and module-root collection passes.
//! WHY: duplicate diagnostics must identify both the current duplicate and the first declaration
//! without storing test-only logic in the production collector module.

use super::*;
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;

fn location(line: i32, start_column: i32, end_column: i32) -> SourceLocation {
    let mut string_table = StringTable::new();
    SourceLocation::new(
        InternedPath::from_single_str("src/@mod.moth", &mut string_table),
        CharPosition {
            line_number: line,
            char_column: start_column,
        },
        CharPosition {
            line_number: line,
            char_column: end_column,
        },
    )
}

#[test]
fn duplicate_public_export_retains_first_owner_location_across_passes() {
    let mut string_table = StringTable::new();
    let export_name = string_table.intern("greet");
    let source_path = InternedPath::from_single_str("src/greet", &mut string_table);
    let first_location = location(2, 5, 10);
    let duplicate_location = location(7, 12, 17);

    let mut first_pass = PublicExportCollector::default();
    first_pass
        .insert(
            export_name,
            PublicExportTarget::SourceDeclaration {
                path: source_path.clone(),
            },
            first_location.clone(),
            &string_table,
        )
        .expect("first public export should be accepted");

    let existing_locations = FxHashMap::from_iter([(export_name, first_location.clone())]);
    let mut second_pass =
        PublicExportCollector::from_existing(&first_pass.exports, Some(&existing_locations));
    let diagnostic = second_pass
        .insert(
            export_name,
            PublicExportTarget::SourceDeclaration { path: source_path },
            duplicate_location.clone(),
            &string_table,
        )
        .expect_err("the second public export should be rejected");

    assert_eq!(diagnostic.primary_location, duplicate_location);
    assert_eq!(diagnostic.labels[1].location, first_location);
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::DuplicatePublicExport {
            first_location: payload_location,
            ..
        } if payload_location == first_location
    ));
}

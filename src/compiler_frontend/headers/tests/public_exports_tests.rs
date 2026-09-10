//! Public-export collector regressions.
//!
//! WHAT: verifies that duplicate public exports retain the first authored owner across the
//! source-package and module-root collection passes.
//! WHY: duplicate diagnostics must identify both the current duplicate and the first declaration
//! without storing test-only logic in the production collector module.

use super::*;
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::headers::types::HeaderParseFailure;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};

fn span(start: u32, length: u32, builder: &mut ExtendedSpanBuilder) -> SourceSpan {
    SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(start, length, builder).expect("test span should fit"),
    )
}

#[test]
fn duplicate_public_export_retains_first_owner_span_across_passes() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let export_name = string_table.intern("greet");
    let source_path = InternedPath::from_single_str("src/greet", &mut string_table);
    let first_span = span(5, 5, &mut span_builder);
    let duplicate_span = span(12, 5, &mut span_builder);

    let mut first_pass = PublicExportCollector::default();
    first_pass
        .insert(
            export_name,
            PublicExportTarget::SourceDeclaration {
                path: source_path.clone(),
            },
            Some(first_span),
            &string_table,
        )
        .expect("first public export should be accepted");

    let existing_spans = FxHashMap::from_iter([(export_name, first_span)]);
    let mut second_pass =
        PublicExportCollector::from_existing(&first_pass.exports, Some(&existing_spans));
    let HeaderParseFailure::Diagnostic(diagnostic) = second_pass
        .insert(
            export_name,
            PublicExportTarget::SourceDeclaration { path: source_path },
            Some(duplicate_span),
            &string_table,
        )
        .expect_err("the second public export should be rejected")
    else {
        panic!("duplicate public export must be a source diagnostic");
    };

    assert_eq!(diagnostic.primary_span, Some(duplicate_span));
    assert_eq!(diagnostic.labels[0].span, Some(first_span));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::DuplicatePublicExport { .. }
    ));
}

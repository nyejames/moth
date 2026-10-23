//! Family render contracts through the public terse entry.
//!
//! WHAT: pins how a severity override on a family kind reaches the rendered record.
//! WHY: the compact path reads severity from the stored record, so an override must win over the
//! family default while the rest of the line stays unchanged.

use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::compact_for_render;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_with_context;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn severity_override_on_a_family_changes_only_the_severity_letter() {
    let string_table = StringTable::new();
    let family_default = CompilerDiagnostic::unreachable_match_arm(None);
    let mut escalated = CompilerDiagnostic::unreachable_match_arm(None);
    escalated.severity = DiagnosticSeverity::Error;

    // Both lines must come from the compact path, or the comparison would not test a stored severity.
    assert!(
        compact_for_render(&family_default).is_some(),
        "the family default must take the compact path"
    );
    assert!(
        compact_for_render(&escalated).is_some(),
        "the escalated diagnostic must take the compact path"
    );

    let default_line = format_terse_diagnostic_with_context(
        &family_default,
        DiagnosticRenderContext::new(&string_table),
    );
    let escalated_line = format_terse_diagnostic_with_context(
        &escalated,
        DiagnosticRenderContext::new(&string_table),
    );

    assert!(
        default_line.starts_with("W|"),
        "the family default is a warning: {default_line}"
    );
    assert!(
        escalated_line.starts_with("E|"),
        "the override must win over the family default: {escalated_line}"
    );
    assert_eq!(
        &default_line[1..],
        &escalated_line[1..],
        "an override may change only the severity letter"
    );
}

//! Content-source ordering hints recorded from declaration shells.
//!
//! WHAT: verifies that every pre-body declaration shell records one content ordering hint per
//!       content-class file-value occurrence, and that runtime bodies, resource paths and
//!       dependency-clause rows record none.
//! WHY: direct `.mtf`/`.md` value paths reuse the synthetic `content` constant, so the shells that
//!       fold before body emission must carry the ordering fact at token level.

use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::parse_file_headers::{
    Header, HeaderKind, LocalDeclarationOrderingHint, prepare_file_from_tokens,
};
use crate::compiler_frontend::headers::types::HeaderParseOptions;
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::collections::HashSet;
use std::path::Path;

fn prepare_source(
    source: &str,
) -> (
    crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let file_path = Path::new("@page.moth");
    let interned_path = InternedPath::try_from_filesystem_path(file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        Some(FileId(0)),
    )
    .expect("tokenization should succeed");
    let output = prepare_file_from_tokens(
        file_tokens,
        file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
    )
    .expect("preparation should succeed");
    (output, string_table)
}

fn content_hint(
    path_text: &str,
    occurrence: PathSyntaxId,
    strings: &mut StringTable,
) -> LocalDeclarationOrderingHint {
    let components = path_text
        .split('/')
        .map(|component| strings.intern(component))
        .collect::<Vec<_>>();
    LocalDeclarationOrderingHint::content_source(
        InternedPath::from_components(components),
        occurrence,
    )
}

fn occurrences_of_class(
    output: &crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    class: PreparedFileReferenceClass,
) -> Vec<PathSyntaxId> {
    output
        .structural_file_references
        .iter()
        .filter(|reference| reference.class == class)
        .map(|reference| reference.path_syntax)
        .collect()
}

fn header_of_kind<'a>(
    headers: &'a [Header],
    kind_label: &str,
    kind_matches: impl Fn(&HeaderKind) -> bool,
) -> &'a Header {
    headers
        .iter()
        .find(|header| kind_matches(&header.kind))
        .unwrap_or_else(|| panic!("expected one {kind_label} header"))
}

fn assert_hints(header: &Header, expected: &HashSet<LocalDeclarationOrderingHint>) {
    assert_eq!(
        &header.local_ordering_hints, expected,
        "unexpected ordering hints on the declaration shell"
    );
}

#[test]
fn constant_initializer_content_value_records_content_hint() {
    let (output, mut strings) = prepare_source("intro #= @docs/intro.mtf\n");

    let header = header_of_kind(&output.headers, "constant", |kind| {
        matches!(kind, HeaderKind::Constant { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.mtf/content",
        occurrences[0],
        &mut strings,
    ));
    assert_hints(header, &expected);
}

#[test]
fn repeated_content_values_in_one_shell_retain_each_occurrence_hint() {
    let (output, mut strings) = prepare_source("#[: [@docs/intro.mtf] [@docs/intro.mtf]]\n");

    let fragment_header = header_of_kind(&output.headers, "const-template", |kind| {
        matches!(kind, HeaderKind::ConstTemplate { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 2);
    let mut expected = HashSet::new();
    for occurrence in occurrences {
        expected.insert(content_hint(
            "docs/intro.mtf/content",
            occurrence,
            &mut strings,
        ));
    }
    assert_hints(fragment_header, &expected);
}

#[test]
fn function_parameter_default_records_content_hint_and_body_stays_unhinted() {
    let (output, mut strings) = prepare_source(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n\
         \x20   io.line([: [@docs/body.md]])\n\
         ;\n",
    );

    let function_header = header_of_kind(&output.headers, "function", |kind| {
        matches!(kind, HeaderKind::Function { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 2);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.md/content",
        occurrences[0],
        &mut strings,
    ));
    assert_hints(function_header, &expected);
}

#[test]
fn struct_field_default_records_content_hint() {
    let (output, mut strings) =
        prepare_source("Config = |\n    path String = [: [@docs/intro.md] ],\n|\n");

    let struct_header = header_of_kind(&output.headers, "struct", |kind| {
        matches!(kind, HeaderKind::Struct { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.md/content",
        occurrences[0],
        &mut strings,
    ));
    assert_hints(struct_header, &expected);
}

#[test]
fn top_level_const_fragment_records_content_hint() {
    let (output, mut strings) = prepare_source("#[: [@docs/intro.md]]\n");

    let fragment_header = header_of_kind(&output.headers, "const-template", |kind| {
        matches!(kind, HeaderKind::ConstTemplate { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.md/content",
        occurrences[0],
        &mut strings,
    ));
    assert_hints(fragment_header, &expected);
}

#[test]
fn runtime_start_body_content_value_records_no_content_hint() {
    let (output, _) = prepare_source("io.line([: [@docs/intro.mtf]])\n");

    let start_header = header_of_kind(&output.headers, "start", |kind| {
        matches!(kind, HeaderKind::StartFunction)
    });

    // The start body must genuinely contain a classified content occurrence, so the no-hint
    // assertion below cannot pass on a shape the collector never visits.
    assert_eq!(
        occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource).len(),
        1,
        "the start body should carry exactly one classified content occurrence"
    );
    assert!(
        start_header.local_ordering_hints.is_empty(),
        "runtime start bodies fold after content constants and must carry no content hints"
    );
}

#[test]
fn resource_file_value_records_no_content_hint() {
    let (output, _) = prepare_source("icon #= @assets/logo.svg\n");

    let header = header_of_kind(&output.headers, "constant", |kind| {
        matches!(kind, HeaderKind::Constant { .. })
    });

    assert!(
        header.local_ordering_hints.is_empty(),
        "resource-only file values resolve through Stage 0 without a content ordering edge"
    );
}

#[test]
fn struct_field_resource_default_records_no_content_hint() {
    let (output, strings) =
        prepare_source("Config = |\n    icon_url String = @assets/logo.svg,\n|\n");

    let struct_header = header_of_kind(&output.headers, "struct", |kind| {
        matches!(kind, HeaderKind::Struct { .. })
    });

    // The field default must genuinely classify as a resource occurrence, so the no-hint
    // assertion below cannot pass on a shape preparation stopped visiting.
    assert_eq!(
        occurrences_of_class(&output, PreparedFileReferenceClass::ResourceFile).len(),
        1,
        "the field default should carry exactly one classified resource occurrence"
    );
    assert!(
        struct_header.local_ordering_hints.is_empty(),
        "a resource-only field default needs no content ordering edge, got {:?} in {}",
        struct_header.local_ordering_hints,
        struct_header.tokens.src_path.to_portable_string(&strings)
    );
}

#[test]
fn dependency_clause_rows_record_no_content_hint() {
    let (output, strings) = prepare_source(
        "@core/math sin\n\
         unused #= @assets/logo.svg\n",
    );

    for header in &output.headers {
        assert!(
            header.local_ordering_hints.is_empty(),
            "clause-consumed and resource rows must record no hints, got {:?} on {}",
            header.local_ordering_hints,
            header.tokens.src_path.to_portable_string(&strings)
        );
    }
}

#[test]
fn recollecting_content_hints_deduplicates_into_the_hint_set() {
    // Re-running the collector over the same shells and rows must be a no-op, so no ordering or
    // diagnostic difference can depend on worklist insertion order.
    let (mut output, mut strings) = prepare_source(
        "intro #= @docs/intro.mtf\n\
         other #= @docs/other.mtf\n",
    );

    let before: Vec<usize> = output
        .headers
        .iter()
        .map(|header| header.local_ordering_hints.len())
        .collect();
    collect_content_source_ordering_hints(
        &mut output.headers,
        &output.structural_file_references,
        &mut strings,
    );
    let after: Vec<usize> = output
        .headers
        .iter()
        .map(|header| header.local_ordering_hints.len())
        .collect();

    assert_eq!(
        before, after,
        "re-collection must deduplicate into the hint set"
    );
    let constants = output
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::Constant { .. }));
    assert!(
        constants
            .map(|header| header.local_ordering_hints.len())
            .eq(std::iter::repeat_n(1usize, 2)),
        "each shell with one content value should hold exactly one hint"
    );
}

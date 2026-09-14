//! Plain Markdown header preparation tests.
//!
//! WHAT: verifies that `.md` files enter the frontend as one normal private `content #String`
//! constant whose rendered HTML is an explicit synthetic payload.
//! WHY: Markdown must not be tokenized or parsed as Moth, so the preparation output shape
//!      is the primary regression surface.

use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::headers::plain_markdown_prepare::{
    PlainMarkdownPrepareInput, prepare_plain_markdown_file,
};
use crate::compiler_frontend::headers::types::{
    FileRole, HeaderExportMode, HeaderKind, SyntheticContentPayload,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn prepare(
    source: &str,
) -> (
    crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    StringTable,
    PathInternerFork,
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork.try_intern_portable_path("docs/intro.md", &mut string_table).expect("test path fits");

    let output = prepare_plain_markdown_file(
        PlainMarkdownPrepareInput {
            source_code: source,
            source_file: source_path,
            file_id: SourceId::COMPILATION_ROOT,
            canonical_os_path: None,
        },
        &mut string_table,
        &mut path_fork,
    )
    .expect("plain Markdown preparation should succeed");

    (output, string_table, path_fork)
}

#[test]
fn produces_exactly_one_header() {
    let (output, _string_table, _path_fork) = prepare("# Heading");

    assert_eq!(output.headers.len(), 1);
    assert!(
        output.source_token_stream.is_none(),
        "plain Markdown must retain the no-token preparation path"
    );
    assert!(
        output.path_syntax.table().paths().is_empty(),
        "plain Markdown must not retain authored path syntax rows"
    );
    assert_eq!(output.token_count, 0, "Markdown files are not tokenized");
    assert_eq!(output.file_role, FileRole::Normal);
    assert!(output.file_dependency_clauses.is_empty());
    assert!(output.top_level_const_fragments.is_empty());
    assert_eq!(output.const_template_count, 0);
    assert_eq!(output.runtime_fragment_count, 0);
    assert!(output.warnings.is_empty());
}

#[test]
fn generated_header_path_ends_with_content() {
    let (output, string_table, path_fork) = prepare("# Heading");

    let header = &output.headers[0];
    let terminal_component = path_fork
        .try_component(header.declaration_path)
        .expect("generated header path must have a terminal component");
    let header_component = string_table.resolve(terminal_component);
    assert_eq!(
        header_component, "content",
        "expected header path terminal component to be content, got {header_component}"
    );
}

#[test]
fn declaration_is_private_compile_time_string_constant() {
    let (output, _string_table, _path_fork) = prepare("# Heading");

    let header = &output.headers[0];
    assert!(matches!(header.export_mode, HeaderExportMode::Private));
    assert_eq!(header.file_role, FileRole::Normal);

    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header, got {:?}", header.kind);
    };

    assert_eq!(
        declaration.span, None,
        "the generated Markdown declaration has no authored source span"
    );
    assert!(matches!(
        declaration.binding_mode,
        BindingMode::CompileTimeConstant
    ));
}

#[test]
fn payload_is_rendered_html_without_initializer_tokens() {
    let (output, string_table, _path_fork) = prepare("# Heading");

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };
    assert!(
        declaration.initializer_tokens.is_empty(),
        "payload-only Markdown must not retain parser initializer tokens"
    );
    let Some(SyntheticContentPayload::RenderedHtml(id)) = header.synthetic_content_payload else {
        panic!("expected explicit rendered HTML payload");
    };
    let rendered = string_table.resolve(id);
    assert!(
        rendered.contains("<h1>Heading</h1>"),
        "expected rendered HTML in payload, got: {rendered}"
    );
}

#[test]
fn markdown_looking_syntax_creates_no_initializer_references() {
    let source = "This costs $100 -- not a comment.\n\nLiteral template-looking text: [not_a_template]\n\nRaw Moth-ish block: [: <p>not parsed</p>]";
    let (output, _string_table, _path_fork) = prepare(source);

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };

    assert!(
        declaration.initializer_references.is_empty(),
        "Markdown literals must not scan rendered HTML for symbol references, got {:?}",
        declaration.initializer_references
    );
}

#[test]
fn rendered_html_is_preserved_exactly() {
    let source = "Text with `backticks`, \"quotes\", [brackets], and\nnewlines.";
    let (output, string_table, _path_fork) = prepare(source);

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };
    assert!(declaration.initializer_tokens.is_empty());
    let Some(SyntheticContentPayload::RenderedHtml(id)) = header.synthetic_content_payload else {
        panic!("expected rendered HTML payload");
    };
    let rendered = string_table.resolve(id);

    let expected =
        "<p>Text with <code>backticks</code>, \"quotes\", [brackets], and\nnewlines.</p>\n";
    assert_eq!(
        rendered, expected,
        "rendered HTML must be preserved exactly in the payload"
    );
}

// -----------------------------------------------------------------------------
// Plain Markdown must not use the template/TIR construction path
// -----------------------------------------------------------------------------

#[test]
fn initializer_contains_no_template_tokens() {
    let source = "# Heading\n\n[not_a_template]\n\n[:not_parsed]";
    let (output, _string_table, _path_fork) = prepare(source);

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };
    assert!(
        declaration.initializer_tokens.is_empty(),
        "plain Markdown must not retain parser initializer tokens"
    );
    assert!(
        matches!(
            header.synthetic_content_payload,
            Some(SyntheticContentPayload::RenderedHtml(_))
        ),
        "plain Markdown must carry explicit rendered HTML payload"
    );
}

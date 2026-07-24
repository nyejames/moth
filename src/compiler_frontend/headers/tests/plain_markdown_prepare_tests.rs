//! Plain Markdown header preparation tests.
//!
//! WHAT: verifies that `.md` files enter the frontend as one normal private `content #String`
//! constant whose initializer is a single literal token holding the rendered HTML.
//! WHY: Markdown must not be tokenized or parsed as Moth, so the preparation output shape
//!      is the primary regression surface.

use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::headers::plain_markdown_prepare::{
    PlainMarkdownPrepareInput, prepare_plain_markdown_file,
};
use crate::compiler_frontend::headers::types::{FileRole, HeaderExportMode, HeaderKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenKind;

fn prepare(
    source: &str,
) -> (
    crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("docs/intro.md", &mut string_table);

    let output = prepare_plain_markdown_file(
        PlainMarkdownPrepareInput {
            source_code: source,
            source_file: source_path,
            file_id: None,
            canonical_os_path: None,
        },
        &mut string_table,
    );

    (output, string_table)
}

#[test]
fn produces_exactly_one_header() {
    let (output, _string_table) = prepare("# Heading");

    assert_eq!(output.headers.len(), 1);
    assert_eq!(output.token_count, 0, "Markdown files are not tokenized");
    assert_eq!(output.file_role, FileRole::Normal);
    assert!(output.file_imports.is_empty());
    assert!(output.top_level_const_fragments.is_empty());
    assert_eq!(output.const_template_count, 0);
    assert_eq!(output.runtime_fragment_count, 0);
    assert!(output.warnings.is_empty());
}

#[test]
fn generated_header_path_ends_with_content() {
    let (output, string_table) = prepare("# Heading");

    let header = &output.headers[0];
    let header_path = header.tokens.src_path.to_portable_string(&string_table);
    assert!(
        header_path.ends_with("content"),
        "expected header path to end with content, got {header_path}"
    );
}

#[test]
fn declaration_is_private_compile_time_string_constant() {
    let (output, _string_table) = prepare("# Heading");

    let header = &output.headers[0];
    assert!(matches!(header.export_mode, HeaderExportMode::Private));
    assert_eq!(header.file_role, FileRole::Normal);

    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header, got {:?}", header.kind);
    };

    assert!(matches!(
        declaration.binding_mode,
        BindingMode::CompileTimeConstant
    ));
    assert!(
        matches!(
            declaration.type_annotation,
            ParsedTypeRef::BuiltinString { .. }
        ),
        "expected builtin String annotation"
    );
}

#[test]
fn initializer_is_single_string_literal_with_rendered_html() {
    let (output, string_table) = prepare("# Heading");

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };

    assert_eq!(declaration.initializer_tokens.len(), 1);
    let token = &declaration.initializer_tokens[0];
    let rendered = match &token.kind {
        TokenKind::StringSliceLiteral(id) => string_table.resolve(*id),
        other => panic!("expected single StringSliceLiteral initializer, got {other:?}"),
    };

    assert!(
        rendered.contains("<h1>Heading</h1>"),
        "expected rendered HTML in literal, got: {rendered}"
    );
}

#[test]
fn markdown_looking_syntax_creates_no_initializer_references() {
    let source = "This costs $100 -- not a comment.\n\nLiteral template-looking text: [not_a_template]\n\nRaw Moth-ish block: [: <p>not parsed</p>]";
    let (output, _string_table) = prepare(source);

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
    let (output, string_table) = prepare(source);

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };

    let token = &declaration.initializer_tokens[0];
    let rendered = match &token.kind {
        TokenKind::StringSliceLiteral(id) => string_table.resolve(*id),
        other => panic!("expected StringSliceLiteral initializer, got {other:?}"),
    };

    let expected =
        "<p>Text with <code>backticks</code>, \"quotes\", [brackets], and\nnewlines.</p>\n";
    assert_eq!(
        rendered, expected,
        "rendered HTML must be preserved exactly in the literal token"
    );
}

// -----------------------------------------------------------------------------
// Plain Markdown must not use the template/TIR construction path
// -----------------------------------------------------------------------------

#[test]
fn initializer_contains_no_template_tokens() {
    let source = "# Heading\n\n[not_a_template]\n\n[:not_parsed]";
    let (output, _string_table) = prepare(source);

    let header = &output.headers[0];
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected constant header");
    };

    assert_eq!(
        declaration.initializer_tokens.len(),
        1,
        "plain Markdown must produce exactly one literal token"
    );

    let forbidden_template_token = declaration.initializer_tokens.iter().any(|token| {
        matches!(
            token.kind,
            TokenKind::TemplateHead
                | TokenKind::StartTemplateBody
                | TokenKind::TemplateClose
                | TokenKind::StyleDirective(_)
        )
    });
    assert!(
        !forbidden_template_token,
        "plain Markdown initializer must not contain template construction tokens"
    );
}

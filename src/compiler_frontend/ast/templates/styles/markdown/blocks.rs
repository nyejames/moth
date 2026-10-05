//! Markdown block rendering: paragraphs, lists, and headings.
//!
//! WHAT: renders block-level markdown constructs from line streams.
//! WHY: separating block logic from inline and parsing keeps each module focused
//!      on one level of the markdown formatter. Plain regions and list items share
//!      one paragraph-grouping path so child-template line rules cannot drift apart.

use super::output::MarkdownOutputBuilder;
use super::parsing::{
    join_paragraph_lines, parse_heading_line, parse_list_item_line, trim_atoms,
    trim_leading_horizontal_whitespace,
};
use super::{MarkdownInlineAtom, MarkdownLine, MarkdownListKind, ParsedMarkdownHeadingLine};
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterOpaqueKind, FormatterOutputPiece,
};

/// One inline-rendered unit inside a plain region or a list item.
enum MarkdownInlineBlock<'a> {
    Paragraph(Vec<&'a [MarkdownInlineAtom]>),
    StandaloneInline(&'a [MarkdownInlineAtom]),
    NestedList(Vec<FormatterOutputPiece>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeadingChildTemplateLine {
    None,
    Standalone,
    InlineContinuation,
}

/// Groups consecutive lines into paragraphs, applying child-template line boundaries.
///
/// WHAT:
/// - Consecutive text/dynamic lines stay in the current paragraph and later join with soft
///   line boundaries.
/// - A child-template anchor alone at line start becomes standalone output.
/// - A child-template anchor followed by same-line content starts a fresh paragraph.
///
/// WHY:
/// - `$md` needs to keep child templates opaque while still letting a single
///   newline before a child break paragraph context without splitting inline helpers
///   from their same-line text.
#[derive(Default)]
struct InlineBlockBuilder<'a> {
    blocks: Vec<MarkdownInlineBlock<'a>>,
    paragraph_lines: Vec<&'a [MarkdownInlineAtom]>,
}

impl<'a> InlineBlockBuilder<'a> {
    fn push_line(&mut self, atoms: &'a [MarkdownInlineAtom]) {
        match classify_leading_child_template_line(atoms) {
            LeadingChildTemplateLine::Standalone => {
                self.flush_paragraph();
                self.blocks.push(MarkdownInlineBlock::StandaloneInline(
                    trim_leading_horizontal_whitespace(atoms),
                ));
            }

            LeadingChildTemplateLine::InlineContinuation => {
                self.flush_paragraph();
                self.blocks.push(MarkdownInlineBlock::Paragraph(vec![
                    trim_leading_horizontal_whitespace(atoms),
                ]));
            }

            LeadingChildTemplateLine::None => self.paragraph_lines.push(atoms),
        }
    }

    fn push_nested_list(&mut self, pieces: Vec<FormatterOutputPiece>) {
        self.flush_paragraph();
        self.blocks.push(MarkdownInlineBlock::NestedList(pieces));
    }

    fn flush_paragraph(&mut self) {
        if !self.paragraph_lines.is_empty() {
            let lines = std::mem::take(&mut self.paragraph_lines);
            self.blocks.push(MarkdownInlineBlock::Paragraph(lines));
        }
    }

    fn finish(mut self) -> Vec<MarkdownInlineBlock<'a>> {
        self.flush_paragraph();
        self.blocks
    }
}

/// Renders a run of plain non-list, non-heading lines as paragraphs and standalone anchors.
pub(super) fn render_plain_region(
    lines: &[MarkdownLine],
    default_tag: &str,
) -> Vec<FormatterOutputPiece> {
    let mut builder = InlineBlockBuilder::default();
    for line in lines {
        builder.push_line(&line.atoms);
    }

    render_inline_blocks(builder.finish(), default_tag, Some(default_tag))
}

/// Parses a list block recursively so nested list items keep mixed content in one `<li>`.
pub(super) fn render_list_block(
    lines: &[MarkdownLine],
    default_tag: &str,
) -> (Vec<FormatterOutputPiece>, usize) {
    struct MarkdownRenderedListSection {
        kind: MarkdownListKind,
        items: Vec<Vec<FormatterOutputPiece>>,
    }

    let Some(first_item) = lines.first().and_then(parse_list_item_line) else {
        return (Vec::new(), 0);
    };
    let current_indent = first_item.indent_width;

    let mut sections: Vec<MarkdownRenderedListSection> = Vec::new();
    let mut consumed_lines = 0usize;

    // Blank and heading lines never parse as list items, so they also end the block here.
    while let Some(list_item) = lines.get(consumed_lines).and_then(parse_list_item_line) {
        if list_item.indent_width != current_indent {
            break;
        }

        consumed_lines += 1;
        let mut item_blocks = InlineBlockBuilder::default();
        item_blocks.push_line(list_item.content);

        // Collect continuation lines and deeper nested lists until the next sibling or outer item.
        while consumed_lines < lines.len() {
            let next_line = &lines[consumed_lines];
            if super::line_is_blank(next_line) || parse_heading_line(next_line).is_some() {
                break;
            }

            if let Some(next_item) = parse_list_item_line(next_line) {
                if next_item.indent_width <= current_indent {
                    break;
                }

                let (nested_rendered, nested_consumed) =
                    render_list_block(&lines[consumed_lines..], default_tag);
                if nested_consumed == 0 {
                    break;
                }

                item_blocks.push_nested_list(nested_rendered);
                consumed_lines += nested_consumed;
                continue;
            }

            item_blocks.push_line(trim_atoms(&next_line.atoms));
            consumed_lines += 1;
        }

        let rendered_item = render_list_item(item_blocks.finish(), default_tag);

        // Adjacent items of a different list kind start a new sibling list.
        match sections.last_mut() {
            Some(section) if section.kind == list_item.kind => section.items.push(rendered_item),
            _ => sections.push(MarkdownRenderedListSection {
                kind: list_item.kind,
                items: vec![rendered_item],
            }),
        }
    }

    let mut output = MarkdownOutputBuilder::default();
    for section in sections {
        output.push_raw(section.kind.open_tag());

        for item in section.items {
            output.push_raw("<li>");
            output.append_pieces(item);
            output.push_raw("</li>");
        }

        output.push_raw(section.kind.close_tag());
    }

    (output.finish(), consumed_lines)
}

/// Renders one list item. A lone paragraph stays unwrapped so simple items read as `<li>text</li>`.
fn render_list_item(
    blocks: Vec<MarkdownInlineBlock<'_>>,
    default_tag: &str,
) -> Vec<FormatterOutputPiece> {
    let paragraph_block_count = blocks
        .iter()
        .filter(|block| matches!(block, MarkdownInlineBlock::Paragraph(_)))
        .count();
    let has_standalone_block = blocks
        .iter()
        .any(|block| matches!(block, MarkdownInlineBlock::StandaloneInline(_)));

    let paragraph_tag = if paragraph_block_count > 1 || has_standalone_block {
        Some(default_tag)
    } else {
        None
    };

    render_inline_blocks(blocks, default_tag, paragraph_tag)
}

/// Renders grouped blocks. `paragraph_tag` wraps paragraphs, while standalone anchor lines
/// only open `default_tag` lazily when same-line text follows the anchor.
fn render_inline_blocks(
    blocks: Vec<MarkdownInlineBlock<'_>>,
    default_tag: &str,
    paragraph_tag: Option<&str>,
) -> Vec<FormatterOutputPiece> {
    let mut output = MarkdownOutputBuilder::default();

    for block in blocks {
        match block {
            MarkdownInlineBlock::Paragraph(lines) => {
                let atoms = join_paragraph_lines(&lines);
                output.append_pieces(super::inline::render_inline_atoms(
                    &atoms,
                    paragraph_tag,
                    paragraph_tag.is_some(),
                ));
            }

            MarkdownInlineBlock::StandaloneInline(atoms) => {
                output.append_pieces(super::inline::render_inline_atoms(
                    atoms,
                    Some(default_tag),
                    false,
                ));
            }

            MarkdownInlineBlock::NestedList(pieces) => output.append_pieces(pieces),
        }
    }

    output.finish()
}

/// Renders a heading with its automatic fragment ID when the whole label is static.
pub(super) fn render_heading_line(
    output: &mut MarkdownOutputBuilder,
    heading: &ParsedMarkdownHeadingLine<'_>,
) {
    let level = heading.level;
    let content = super::inline::render_heading_content(heading.content);

    output.push_raw(&format!("<h{level}"));
    if let Some(id) = &content.id {
        // IDs contain only letters, digits and `-`, so they need no attribute escaping.
        output.push_raw(" id=\"");
        output.push_raw(id);
        output.push_raw("\"");
    }
    output.push_raw(">");
    output.append_pieces(content.pieces);
    output.push_raw(&format!("</h{level}>"));
}

fn classify_leading_child_template_line(atoms: &[MarkdownInlineAtom]) -> LeadingChildTemplateLine {
    let anchor_index = super::parsing::skip_leading_horizontal_whitespace(atoms);

    let Some(MarkdownInlineAtom::Opaque(anchor)) = atoms.get(anchor_index) else {
        return LeadingChildTemplateLine::None;
    };

    if anchor.kind != FormatterOpaqueKind::ChildTemplate {
        return LeadingChildTemplateLine::None;
    }

    let has_trailing_content = atoms[anchor_index + 1..].iter().any(|atom| match atom {
        MarkdownInlineAtom::Char(ch) => !matches!(ch, ' ' | '\t' | '\r' | '\n'),
        MarkdownInlineAtom::Opaque(_) => true,
    });

    if has_trailing_content {
        LeadingChildTemplateLine::InlineContinuation
    } else {
        LeadingChildTemplateLine::Standalone
    }
}

//! Markdown block rendering: paragraphs, lists, and headings.
//!
//! WHAT: renders block-level markdown constructs from line streams.
//! WHY: separating block logic from inline and parsing keeps each module focused
//!      on one level of the markdown formatter. Plain regions and list items share
//!      one paragraph-grouping path so child-template line rules cannot drift apart.

use super::output::MarkdownOutputBuilder;
use super::parsing::{join_paragraph_lines, parse_heading_line, parse_list_item_line};
use super::{
    MarkdownAtomsRef, MarkdownInlineAtom, MarkdownLine, MarkdownListKind,
    ParsedMarkdownHeadingLine, ParsedMarkdownMarker,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterOpaqueKind, FormatterOutputPiece,
};

/// One inline-rendered unit inside a plain region or a list item.
enum MarkdownInlineBlock<'a> {
    Paragraph(Vec<MarkdownAtomsRef<'a>>),
    StandaloneInline(MarkdownAtomsRef<'a>),
    NestedList(Vec<FormatterOutputPiece>),
}

/// Whether each paragraph owns its marker ID or an enclosing list item collects them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockMarkerMode {
    PerParagraph,
    CollectForListItem,
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
    paragraph_lines: Vec<MarkdownAtomsRef<'a>>,
}

impl<'a> InlineBlockBuilder<'a> {
    fn push_line(&mut self, atoms: MarkdownAtomsRef<'a>) {
        match classify_leading_child_template_line(atoms) {
            LeadingChildTemplateLine::Standalone => {
                self.flush_paragraph();
                self.blocks.push(MarkdownInlineBlock::StandaloneInline(
                    atoms.trim_leading_horizontal_whitespace(),
                ));
            }

            LeadingChildTemplateLine::InlineContinuation => {
                self.flush_paragraph();
                self.blocks.push(MarkdownInlineBlock::Paragraph(vec![
                    atoms.trim_leading_horizontal_whitespace(),
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
) -> Result<Vec<FormatterOutputPiece>, TemplateError> {
    let mut builder = InlineBlockBuilder::default();
    for line in lines {
        builder.push_line(MarkdownAtomsRef::whole(line));
    }

    let rendered = render_inline_blocks(
        builder.finish(),
        Some(default_tag),
        BlockMarkerMode::PerParagraph,
    )?;
    Ok(rendered.pieces)
}

/// Parses a list block recursively so nested list items keep mixed content in one `<li>`.
pub(super) fn render_list_block(
    lines: &[MarkdownLine],
    default_tag: &str,
) -> Result<(Vec<FormatterOutputPiece>, usize), TemplateError> {
    struct MarkdownRenderedListSection {
        kind: MarkdownListKind,
        items: Vec<RenderedListItem>,
    }

    let Some(first_item) = lines.first().and_then(parse_list_item_line) else {
        return Ok((Vec::new(), 0));
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
                    render_list_block(&lines[consumed_lines..], default_tag)?;
                if nested_consumed == 0 {
                    break;
                }

                item_blocks.push_nested_list(nested_rendered);
                consumed_lines += nested_consumed;
                continue;
            }

            item_blocks.push_line(MarkdownAtomsRef::whole(next_line).trimmed());
            consumed_lines += 1;
        }

        let rendered_item = render_list_item(item_blocks.finish(), default_tag)?;

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
            output.push_raw("<li");
            if let Some(marker) = &item.marker {
                push_id_attribute(&mut output, &marker.id);
            }
            output.push_raw(">");
            output.append_pieces(item.pieces);
            output.push_raw("</li>");
        }

        output.push_raw(section.kind.close_tag());
    }

    Ok((output.finish(), consumed_lines))
}

/// Rendered list item content plus the marker that gives the enclosing `<li>` its ID.
struct RenderedListItem {
    pieces: Vec<FormatterOutputPiece>,
    marker: Option<ParsedMarkdownMarker>,
}

/// Renders one list item. A lone paragraph stays unwrapped so simple items read as `<li>text</li>`.
fn render_list_item(
    blocks: Vec<MarkdownInlineBlock<'_>>,
    default_tag: &str,
) -> Result<RenderedListItem, TemplateError> {
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

    let rendered =
        render_inline_blocks(blocks, paragraph_tag, BlockMarkerMode::CollectForListItem)?;
    ensure_at_most_one_marker(&rendered.markers)?;

    Ok(RenderedListItem {
        pieces: rendered.pieces,
        marker: rendered.markers.into_iter().next(),
    })
}

struct RenderedInlineBlocks {
    pieces: Vec<FormatterOutputPiece>,
    markers: Vec<ParsedMarkdownMarker>,
}

/// Renders grouped blocks. `paragraph_tag` wraps paragraphs; standalone child-anchor lines
/// stay unwrapped. Per-paragraph mode validates each paragraph's marker and drops the fact;
/// list-item mode returns the markers so the enclosing `<li>` can own them.
fn render_inline_blocks(
    blocks: Vec<MarkdownInlineBlock<'_>>,
    paragraph_tag: Option<&str>,
    marker_mode: BlockMarkerMode,
) -> Result<RenderedInlineBlocks, TemplateError> {
    let mut output = MarkdownOutputBuilder::default();
    let mut collected_markers = Vec::new();

    for block in blocks {
        match block {
            MarkdownInlineBlock::Paragraph(lines) => {
                let atoms = join_paragraph_lines(&lines);
                let rendered = super::inline::render_inline_atoms(atoms.view())?;
                if marker_mode == BlockMarkerMode::PerParagraph {
                    ensure_at_most_one_marker(&rendered.markers)?;
                    append_owned_paragraph(&mut output, rendered, paragraph_tag);
                } else {
                    collected_markers.extend(rendered.markers);
                    append_unowned_paragraph(&mut output, rendered.pieces, paragraph_tag);
                }
            }

            // The grouping pass only emits this form when the line held a child anchor
            // and no same-line content; markers count as content, so they cannot appear
            // here. It is deliberately unwrapped so a direct child template never gains
            // a paragraph its author did not write.
            MarkdownInlineBlock::StandaloneInline(atoms) => {
                let rendered = super::inline::render_inline_atoms(atoms)?;
                output.append_pieces(rendered.pieces);
            }

            MarkdownInlineBlock::NestedList(pieces) => output.append_pieces(pieces),
        }
    }

    Ok(RenderedInlineBlocks {
        pieces: output.finish(),
        markers: collected_markers,
    })
}

/// Appends a paragraph that owns its marker. An empty paragraph leaves no element, so an
/// authored marker emits a standalone empty span instead; otherwise the paragraph carries
/// the ID and no span is added.
fn append_owned_paragraph(
    output: &mut MarkdownOutputBuilder,
    rendered: super::inline::RenderedInline,
    paragraph_tag: Option<&str>,
) {
    let Some(marker) = rendered.markers.first() else {
        append_unowned_paragraph(output, rendered.pieces, paragraph_tag);
        return;
    };

    if rendered.pieces.is_empty() {
        push_standalone_span(output, marker);
        return;
    }

    if let Some(tag) = paragraph_tag {
        output.push_raw(&format!("<{tag}"));
        push_id_attribute(output, &marker.id);
        output.push_raw(">");
    }
    output.append_pieces(rendered.pieces);
    if let Some(tag) = paragraph_tag {
        output.push_raw(&format!("</{tag}>"));
    }
}

fn append_unowned_paragraph(
    output: &mut MarkdownOutputBuilder,
    pieces: Vec<FormatterOutputPiece>,
    paragraph_tag: Option<&str>,
) {
    if let Some(tag) = paragraph_tag {
        output.push_raw(&format!("<{tag}>"));
    }
    output.append_pieces(pieces);
    if let Some(tag) = paragraph_tag {
        output.push_raw(&format!("</{tag}>"));
    }
}

/// Renders a heading. A marker assigns its ID; there is no automatic ID.
pub(super) fn render_heading_line(
    output: &mut MarkdownOutputBuilder,
    heading: &ParsedMarkdownHeadingLine<'_>,
) -> Result<(), TemplateError> {
    let level = heading.level;
    let content = super::inline::render_inline_atoms(heading.content)?;
    ensure_at_most_one_marker(&content.markers)?;

    output.push_raw(&format!("<h{level}"));
    if let Some(marker) = content.markers.first() {
        push_id_attribute(output, &marker.id);
    }
    output.push_raw(">");
    output.append_pieces(content.pieces);
    output.push_raw(&format!("</h{level}>"));
    Ok(())
}

/// Writes an escaped ` id="…"` attribute.
fn push_id_attribute(output: &mut MarkdownOutputBuilder, id: &str) {
    output.push_raw(" id=\"");
    output.push_escaped_text(id);
    output.push_raw("\"");
}

/// Emits the standalone empty span for an authored marker with no owning element.
fn push_standalone_span(output: &mut MarkdownOutputBuilder, marker: &ParsedMarkdownMarker) {
    if marker.span.is_none() {
        return;
    }

    output.push_raw("<span");
    push_id_attribute(output, &marker.id);
    output.push_raw("></span>");
}

/// Rejects a second marker on one Markdown-owned block, labelling the first when it has a span.
pub(super) fn ensure_at_most_one_marker(
    markers: &[ParsedMarkdownMarker],
) -> Result<(), TemplateError> {
    let Some(second) = markers.get(1) else {
        return Ok(());
    };

    Err(super::duplicate_section_marker(
        second.span,
        markers[0].span,
    ))
}

fn classify_leading_child_template_line(atoms: MarkdownAtomsRef<'_>) -> LeadingChildTemplateLine {
    let anchor_index = atoms.skip_leading_horizontal_whitespace();

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

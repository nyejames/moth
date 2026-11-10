//! Markdown inline rendering: emphasis, code spans, links and section markers.
//!
//! WHAT: renders inline markdown atoms into escaped HTML and preserved opaque anchors.
//! WHY: inline formatting is the largest single concern in the markdown formatter;
//!      extracting it into its own module keeps the block orchestration readable.
//!      A section marker is zero-width: it records one parsed fact for the owning block and
//!      never opens a pending emphasis or leaves an empty element behind.
//!      The block layer owns wrapper elements and applies any marker ID to them.

use super::inline_code::{ParsedInlineCodeSpan, try_parse_inline_code_span_at_atoms};
use super::parsing::try_parse_reference_at_atoms;
use super::{
    MarkdownAtomsRef, MarkdownInlineAtom, MarkdownOutputBuilder, ParsedMarkdownLink,
    ParsedMarkdownMarker, ParsedMarkdownReference, atom_char,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::formatter_contract::FormatterOutputPiece;

/// Inline-rendered content plus the section markers found in this block.
pub(super) struct RenderedInline {
    pub(super) pieces: Vec<FormatterOutputPiece>,
    pub(super) markers: Vec<ParsedMarkdownMarker>,
}

/// Renders inline markdown atoms into escaped HTML and preserved opaque anchors.
///
/// WHAT:
/// - Escapes text, parses labelled links and bare section markers, and maintains a narrow
///   emphasis state machine across both text and opaque anchors.
///
/// WHY:
/// - Inline formatting needs to stay structurally continuous across child/dynamic
///   anchors without flattening them into temporary strings.
/// - Block wrapping lives in `blocks`: its grouping pass already knows which paragraphs
///   and child-template lines own a wrapper, so the inline walk only renders content.
pub(super) fn render_inline_atoms(
    atoms: MarkdownAtomsRef<'_>,
) -> Result<RenderedInline, TemplateError> {
    let mut renderer = InlineRenderer::new();
    renderer.render_atoms(atoms)?;
    Ok(renderer.finish())
}

/// Output and emphasis state shared by every inline construct in one block.
struct InlineRenderer {
    output: MarkdownOutputBuilder,
    emphasis_strength: Option<usize>,
    pending_open_strength: usize,
    markers: Vec<ParsedMarkdownMarker>,
    /// Whether visible text, a link, code or an opaque anchor has been emitted.
    has_visible_content: bool,
    /// Whether the last emitted character was a block separator. Tags and markers do not
    /// change this; they are not visible separators.
    last_was_separator: bool,
}

impl InlineRenderer {
    fn new() -> Self {
        Self {
            output: MarkdownOutputBuilder::default(),
            emphasis_strength: None,
            pending_open_strength: 0,
            markers: Vec::new(),
            has_visible_content: false,
            last_was_separator: true,
        }
    }

    fn render_atoms(&mut self, atoms: MarkdownAtomsRef<'_>) -> Result<(), TemplateError> {
        let mut prev_whitespace = true;
        let mut atom_index = 0usize;

        while atom_index < atoms.len() {
            // A star run opens emphasis only once visible content follows it. A marker is
            // zero-width, so it never forces the pending run open.
            if self.pending_open_strength > 0
                && !matches!(atom_char(&atoms, atom_index), Some(' ' | '\t' | '*' | '@'))
            {
                self.open_pending_emphasis();
            }

            match atoms[atom_index] {
                MarkdownInlineAtom::Opaque(anchor) => {
                    self.has_visible_content = true;
                    self.last_was_separator = false;
                    self.output.push_opaque(anchor);
                    prev_whitespace = false;
                    atom_index += 1;
                }

                MarkdownInlineAtom::Char(ch @ (' ' | '\t')) => {
                    self.literalize_pending_stars();
                    self.output.push_escaped_char(ch);
                    self.last_was_separator = true;
                    prev_whitespace = true;
                    atom_index += 1;
                }

                MarkdownInlineAtom::Char('\n' | '\r') => {
                    self.literalize_pending_stars();
                    // Soft line boundaries render as one space but remain newline atoms
                    // so inline parsing can apply each construct's newline rule.
                    self.output.push_escaped_char(' ');
                    self.last_was_separator = true;
                    prev_whitespace = true;
                    atom_index += 1;
                }

                MarkdownInlineAtom::Char('*') => {
                    let star_run = count_consecutive_star_chars(&atoms, atom_index);

                    // A marker has no visible content, so a matching run cancels the
                    // emphasis pair rather than leaving empty `<em>` markup. The check is
                    // scoped to the pending run, not to the whole block.
                    // A pending run still open here has seen only zero-width markers:
                    // any visible atom opens or literalizes it before this branch.
                    if self.pending_open_strength > 0 && star_run >= self.pending_open_strength {
                        let after_run = atom_index + self.pending_open_strength;
                        let rest_is_only_separators = (after_run..atoms.len())
                            .all(|index| is_block_separator(atom_char(&atoms, index)));

                        self.pending_open_strength = 0;
                        if rest_is_only_separators {
                            self.output.trim_trailing_horizontal_whitespace();
                            atom_index = atoms.len();
                        } else if self.last_was_separator
                            && is_block_separator(atom_char(&atoms, after_run))
                        {
                            atom_index = after_run + 1;
                        } else {
                            atom_index = after_run;
                        }
                        continue;
                    }

                    if let Some(active_strength) = self.emphasis_strength {
                        if star_run >= active_strength {
                            self.output.push_raw(emphasis_close_tag(active_strength));
                            self.emphasis_strength = None;
                            atom_index += active_strength;
                        } else {
                            self.output.push_raw(&"*".repeat(star_run));
                            self.has_visible_content = true;
                            self.last_was_separator = false;
                            atom_index += star_run;
                        }
                        prev_whitespace = false;
                        continue;
                    }

                    if prev_whitespace && (1..=3).contains(&star_run) {
                        self.pending_open_strength = star_run;
                        atom_index += star_run;
                        continue;
                    }

                    self.literalize_pending_stars();
                    self.output.push_raw(&"*".repeat(star_run));
                    self.has_visible_content = true;
                    self.last_was_separator = false;
                    prev_whitespace = false;
                    atom_index += star_run;
                }

                MarkdownInlineAtom::Char('`') => {
                    if let Some(span) = try_parse_inline_code_span_at_atoms(&atoms, atom_index) {
                        self.literalize_pending_stars();
                        self.push_inline_code_span(&span);
                        prev_whitespace = false;
                        atom_index += span.consumed_atoms;
                        continue;
                    }

                    self.push_visible_char('`');
                    prev_whitespace = false;
                    atom_index += 1;
                }

                MarkdownInlineAtom::Char('@') if prev_whitespace => {
                    let closing_emphasis_strength = self.emphasis_strength.or_else(|| {
                        (self.pending_open_strength > 0).then_some(self.pending_open_strength)
                    });

                    match try_parse_reference_at_atoms(atoms, atom_index, closing_emphasis_strength)
                    {
                        ParsedMarkdownReference::Link(link) => {
                            self.push_link(&link);
                            prev_whitespace = false;
                            atom_index += link.consumed_atoms;
                        }

                        ParsedMarkdownReference::Marker(marker) => {
                            let next_index = atom_index + marker.consumed_atoms;
                            atom_index = self.consume_marker(&marker, atoms, next_index);
                        }

                        ParsedMarkdownReference::InvalidMarker { span } => {
                            return Err(super::invalid_section_marker(span));
                        }

                        // A malformed candidate stays literal text. Only its `@` is consumed so
                        // the rest, including any later link, renders normally.
                        ParsedMarkdownReference::Literal => {
                            self.push_visible_char('@');
                            prev_whitespace = false;
                            atom_index += 1;
                        }
                    }
                }

                MarkdownInlineAtom::Char(ch) => {
                    self.push_visible_char(ch);
                    prev_whitespace = false;
                    atom_index += 1;
                }
            }
        }

        Ok(())
    }

    /// Records one marker and applies its zero-width whitespace rule.
    ///
    /// A marker is absent from rendered text. At a block edge its adjacent separators are
    /// removed; between two separators one is removed so `A @#x B` becomes `A B`.
    fn consume_marker(
        &mut self,
        marker: &ParsedMarkdownMarker,
        atoms: MarkdownAtomsRef<'_>,
        next_index: usize,
    ) -> usize {
        self.markers.push(ParsedMarkdownMarker {
            id: marker.id.clone(),
            span: marker.span,
            consumed_atoms: marker.consumed_atoms,
        });

        let at_block_start = !self.has_visible_content;
        let next_is_separator = is_block_separator(atom_char(&atoms, next_index));
        let rest_is_only_separators =
            (next_index..atoms.len()).all(|index| is_block_separator(atom_char(&atoms, index)));

        if at_block_start || rest_is_only_separators {
            self.output.trim_trailing_horizontal_whitespace();
            if rest_is_only_separators {
                return atoms.len();
            }
            if next_is_separator {
                return next_index + 1;
            }
            return next_index;
        }

        if self.last_was_separator && next_is_separator {
            return next_index + 1;
        }

        next_index
    }

    fn open_pending_emphasis(&mut self) {
        if self.pending_open_strength == 0 {
            return;
        }

        self.output
            .push_raw(emphasis_open_tag(self.pending_open_strength));
        self.emphasis_strength = Some(self.pending_open_strength);
        self.pending_open_strength = 0;
    }

    /// Prints a pending star run as text when whitespace or a construct follows it instead.
    fn literalize_pending_stars(&mut self) {
        if self.pending_open_strength == 0 {
            return;
        }

        self.output
            .push_raw(&"*".repeat(self.pending_open_strength));
        self.pending_open_strength = 0;
        self.has_visible_content = true;
        self.last_was_separator = false;
    }

    fn push_visible_char(&mut self, ch: char) {
        self.open_pending_emphasis();
        self.has_visible_content = true;
        self.last_was_separator = false;
        self.output.push_escaped_char(ch);
    }

    /// Renders one labelled link without changing any enclosing wrapper ownership.
    fn push_link(&mut self, link: &ParsedMarkdownLink) {
        self.open_pending_emphasis();
        self.has_visible_content = true;
        self.last_was_separator = false;
        self.output.push_raw("<a href=\"");

        // Single-slash targets are site routes and render through the configured site root.
        if let Some(route) = link.target.strip_prefix('/')
            && !route.starts_with('/')
        {
            self.output.push_site_root();
            self.output.push_escaped_text(route);
        } else {
            self.output.push_escaped_text(&link.target);
        }

        self.output.push_raw("\">");
        self.output.push_escaped_text(&link.label);
        self.output.push_raw("</a>");
    }

    /// Renders one code span while preserving any opaque anchors inside it.
    fn push_inline_code_span(&mut self, span: &ParsedInlineCodeSpan) {
        self.has_visible_content = true;
        self.last_was_separator = false;
        self.output.push_raw("<code>");

        for content_atom in &span.content {
            match content_atom {
                MarkdownInlineAtom::Char(ch) => {
                    self.output.push_escaped_char(*ch);
                }
                // The parser rejects child-template anchors before returning a span.
                // Opaque anchors that reach rendering are dynamic-expression placeholders
                // and must stay opaque to the parent formatter.
                MarkdownInlineAtom::Opaque(anchor) => {
                    self.output.push_opaque(*anchor);
                }
            }
        }

        self.output.push_raw("</code>");
    }

    fn finish(mut self) -> RenderedInline {
        self.literalize_pending_stars();

        if let Some(active_strength) = self.emphasis_strength {
            self.output.push_raw(emphasis_close_tag(active_strength));
        }

        RenderedInline {
            pieces: self.output.finish(),
            markers: self.markers,
        }
    }
}

fn is_block_separator(ch: Option<char>) -> bool {
    matches!(ch, Some(' ' | '\t' | '\n' | '\r'))
}

fn count_consecutive_star_chars(atoms: &[MarkdownInlineAtom], start_index: usize) -> usize {
    let mut count = 0usize;
    while let Some('*') = atom_char(atoms, start_index + count) {
        count += 1;
    }
    count
}

fn emphasis_open_tag(strength: usize) -> &'static str {
    match strength {
        2 => "<strong>",
        3 => "<em><strong>",
        _ => "<em>",
    }
}

fn emphasis_close_tag(strength: usize) -> &'static str {
    match strength {
        2 => "</strong>",
        3 => "</strong></em>",
        _ => "</em>",
    }
}

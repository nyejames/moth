//! Markdown inline rendering: emphasis, code spans, and links.
//!
//! WHAT: renders inline markdown atoms into escaped HTML and preserved opaque anchors.
//! WHY: inline formatting is the largest single concern in the markdown formatter;
//!      extracting it into its own module keeps the block orchestration readable.

use super::inline_code::{ParsedInlineCodeSpan, try_parse_inline_code_span_at_atoms};
use super::parsing::try_parse_link_at_atoms;
use super::{MarkdownInlineAtom, MarkdownOutputBuilder, ParsedMarkdownLink, atom_char};
use crate::compiler_frontend::ast::templates::formatter_contract::FormatterOutputPiece;

/// Renders inline markdown atoms into escaped HTML and preserved opaque anchors.
///
/// WHAT:
/// - Escapes text, parses the markdown link syntax, and maintains a narrow emphasis
///   state machine across both text and opaque anchors.
/// - Supports lazy wrapper opening so child-template-leading lines can render an
///   anchor first and only open `<p>` when later text appears.
///
/// WHY:
/// - Inline formatting needs to stay structurally continuous across child/dynamic
///   anchors without flattening them into temporary strings.
pub(super) fn render_inline_atoms(
    atoms: &[MarkdownInlineAtom],
    wrapper_tag: Option<&str>,
    open_wrapper_immediately: bool,
) -> Vec<FormatterOutputPiece> {
    let mut renderer = InlineRenderer {
        output: MarkdownOutputBuilder::default(),
        wrapper_tag,
        wrapper_open: false,
        emphasis_strength: None,
        pending_open_strength: 0,
    };
    let mut prev_whitespace = true;
    let mut atom_index = 0usize;

    if open_wrapper_immediately {
        renderer.open_wrapper();
    }

    while atom_index < atoms.len() {
        // A star run opens emphasis only once visible content follows it.
        if renderer.pending_open_strength > 0
            && !matches!(
                atoms[atom_index],
                MarkdownInlineAtom::Char(' ' | '\t' | '*')
            )
        {
            renderer.open_pending_emphasis();
        }

        match atoms[atom_index] {
            MarkdownInlineAtom::Opaque(anchor) => {
                renderer.output.push_opaque(anchor);
                prev_whitespace = false;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char(ch @ (' ' | '\t')) => {
                renderer.literalize_pending_stars();
                renderer.output.push_escaped_char(ch);
                prev_whitespace = true;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char('\n' | '\r') => {
                renderer.literalize_pending_stars();
                // Soft line boundaries render as one space but remain newline atoms
                // so inline parsing can apply each construct's newline rule.
                renderer.output.push_escaped_char(' ');
                prev_whitespace = true;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char('*') => {
                let star_run = count_consecutive_star_chars(atoms, atom_index);

                if let Some(active_strength) = renderer.emphasis_strength {
                    if star_run >= active_strength {
                        renderer
                            .output
                            .push_raw(emphasis_close_tag(active_strength));
                        renderer.emphasis_strength = None;
                        atom_index += active_strength;
                    } else {
                        renderer.open_wrapper();
                        renderer.output.push_raw(&"*".repeat(star_run));
                        atom_index += star_run;
                    }
                    prev_whitespace = false;
                    continue;
                }

                if prev_whitespace && (1..=3).contains(&star_run) {
                    renderer.pending_open_strength = star_run;
                    atom_index += star_run;
                    continue;
                }

                renderer.literalize_pending_stars();
                renderer.open_wrapper();
                renderer.output.push_raw(&"*".repeat(star_run));
                prev_whitespace = false;
                atom_index += star_run;
            }

            MarkdownInlineAtom::Char('`') => {
                if let Some(span) = try_parse_inline_code_span_at_atoms(atoms, atom_index) {
                    renderer.literalize_pending_stars();
                    renderer.open_wrapper();
                    render_inline_code_span(&mut renderer.output, &span);
                    prev_whitespace = false;
                    atom_index += span.consumed_atoms;
                    continue;
                }

                renderer.push_visible_char('`');
                prev_whitespace = false;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char('@') if prev_whitespace => {
                if let Some(link) = try_parse_link_at_atoms(atoms, atom_index) {
                    renderer.push_link(&link);
                    prev_whitespace = false;
                    atom_index += link.consumed_atoms;
                    continue;
                }

                // A malformed candidate stays literal text. Only its `@` is consumed so
                // the rest, including any later link, renders normally.
                renderer.push_visible_char('@');
                prev_whitespace = false;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char(ch) => {
                renderer.push_visible_char(ch);
                prev_whitespace = false;
                atom_index += 1;
            }
        }
    }

    renderer.finish()
}

/// Output and emphasis state shared by every inline construct in one block.
struct InlineRenderer<'tag> {
    output: MarkdownOutputBuilder,
    wrapper_tag: Option<&'tag str>,
    wrapper_open: bool,
    emphasis_strength: Option<usize>,
    pending_open_strength: usize,
}

impl InlineRenderer<'_> {
    fn open_wrapper(&mut self) {
        if self.wrapper_open {
            return;
        }

        if let Some(tag) = self.wrapper_tag {
            self.output.push_raw(&format!("<{tag}>"));
            self.wrapper_open = true;
        }
    }

    fn open_pending_emphasis(&mut self) {
        if self.pending_open_strength == 0 {
            return;
        }

        self.open_wrapper();
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

        self.open_wrapper();
        self.output
            .push_raw(&"*".repeat(self.pending_open_strength));
        self.pending_open_strength = 0;
    }

    fn push_visible_char(&mut self, ch: char) {
        self.open_pending_emphasis();
        self.open_wrapper();
        self.output.push_escaped_char(ch);
    }

    fn push_link(&mut self, link: &ParsedMarkdownLink) {
        self.open_pending_emphasis();
        self.open_wrapper();
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

    fn finish(mut self) -> Vec<FormatterOutputPiece> {
        self.literalize_pending_stars();

        if let Some(active_strength) = self.emphasis_strength {
            self.output.push_raw(emphasis_close_tag(active_strength));
        }

        if self.wrapper_open
            && let Some(tag) = self.wrapper_tag
        {
            self.output.push_raw(&format!("</{tag}>"));
        }

        self.output.finish()
    }
}

fn render_inline_code_span(output: &mut MarkdownOutputBuilder, span: &ParsedInlineCodeSpan) {
    output.push_raw("<code>");

    for content_atom in &span.content {
        match content_atom {
            MarkdownInlineAtom::Char(ch) => output.push_escaped_char(*ch),
            // The parser rejects child-template anchors before returning a span.
            // Opaque anchors that reach rendering are dynamic-expression placeholders
            // and must stay opaque to the parent formatter.
            MarkdownInlineAtom::Opaque(anchor) => output.push_opaque(*anchor),
        }
    }

    output.push_raw("</code>");
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

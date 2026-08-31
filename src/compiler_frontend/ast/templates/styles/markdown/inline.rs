//! Markdown inline rendering: emphasis, code spans, and links.
//!
//! WHAT: renders inline markdown atoms into escaped HTML and preserved opaque anchors.
//! WHY: inline formatting is the largest single concern in the markdown formatter;
//!      extracting it into its own module keeps the block orchestration readable.

use super::{MarkdownInlineAtom, MarkdownOutputBuilder};
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
    let mut output = MarkdownOutputBuilder::default();
    let mut wrapper_open = false;
    let mut emphasis_strength: Option<usize> = None;
    let mut pending_open_strength = 0usize;
    let mut prev_whitespace = true;
    let mut atom_index = 0usize;

    if open_wrapper_immediately {
        open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
    }

    while atom_index < atoms.len() {
        if pending_open_strength > 0
            && !matches!(
                atoms[atom_index],
                MarkdownInlineAtom::Char(' ' | '\t' | '*')
            )
        {
            open_pending_emphasis(
                &mut output,
                wrapper_tag,
                &mut wrapper_open,
                &mut emphasis_strength,
                &mut pending_open_strength,
            );
        }

        match atoms[atom_index] {
            MarkdownInlineAtom::Opaque(anchor) => {
                output.push_opaque(anchor);
                prev_whitespace = false;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char(ch @ (' ' | '\t')) => {
                literalize_pending_stars(
                    &mut output,
                    wrapper_tag,
                    &mut wrapper_open,
                    &mut pending_open_strength,
                );
                output.push_escaped_char(ch);
                prev_whitespace = true;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char('\n') | MarkdownInlineAtom::Char('\r') => {
                literalize_pending_stars(
                    &mut output,
                    wrapper_tag,
                    &mut wrapper_open,
                    &mut pending_open_strength,
                );
                // Soft line boundaries render as one space but remain newline atoms
                // here so inline parsing can still reject cross-line constructs.
                output.push_escaped_char(' ');
                prev_whitespace = true;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char('*') => {
                let star_run = count_consecutive_star_chars(atoms, atom_index);

                if let Some(active_strength) = emphasis_strength {
                    if star_run >= active_strength {
                        output.push_raw(em_tag_strength(active_strength as i32, true));
                        emphasis_strength = None;
                        prev_whitespace = false;
                        atom_index += active_strength;
                    } else {
                        open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                        output.push_raw(&"*".repeat(star_run));
                        prev_whitespace = false;
                        atom_index += star_run;
                    }
                    continue;
                }

                if prev_whitespace && (1..=3).contains(&star_run) {
                    pending_open_strength = star_run;
                    atom_index += star_run;
                    continue;
                }

                literalize_pending_stars(
                    &mut output,
                    wrapper_tag,
                    &mut wrapper_open,
                    &mut pending_open_strength,
                );
                open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                output.push_raw(&"*".repeat(star_run));
                prev_whitespace = false;
                atom_index += star_run;
            }

            MarkdownInlineAtom::Char('`') => {
                if let Some(span) =
                    super::inline_code::try_parse_inline_code_span_at_atoms(atoms, atom_index)
                {
                    literalize_pending_stars(
                        &mut output,
                        wrapper_tag,
                        &mut wrapper_open,
                        &mut pending_open_strength,
                    );
                    open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                    render_inline_code_span(&mut output, &span);
                    prev_whitespace = false;
                    atom_index += span.consumed_atoms;
                    continue;
                }

                open_pending_emphasis(
                    &mut output,
                    wrapper_tag,
                    &mut wrapper_open,
                    &mut emphasis_strength,
                    &mut pending_open_strength,
                );
                open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                output.push_escaped_char('`');
                prev_whitespace = false;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char('@') if prev_whitespace => {
                if let Some(link) = super::parsing::try_parse_link_at_atoms(atoms, atom_index) {
                    open_pending_emphasis(
                        &mut output,
                        wrapper_tag,
                        &mut wrapper_open,
                        &mut emphasis_strength,
                        &mut pending_open_strength,
                    );
                    open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                    output.push_raw("<a href=\"");
                    if link.target.starts_with('/') && !link.target.starts_with("//") {
                        output.push_site_root();
                        output.push_escaped_text(&link.target[1..]);
                    } else {
                        output.push_escaped_text(&link.target);
                    }
                    output.push_raw("\">");
                    output.push_escaped_text(&link.label);
                    output.push_raw("</a>");
                    prev_whitespace = false;
                    atom_index += link.consumed_atoms;
                    continue;
                }

                open_pending_emphasis(
                    &mut output,
                    wrapper_tag,
                    &mut wrapper_open,
                    &mut emphasis_strength,
                    &mut pending_open_strength,
                );
                open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                output.push_escaped_char('@');
                prev_whitespace = false;
                atom_index += 1;
            }

            MarkdownInlineAtom::Char(ch) => {
                open_pending_emphasis(
                    &mut output,
                    wrapper_tag,
                    &mut wrapper_open,
                    &mut emphasis_strength,
                    &mut pending_open_strength,
                );
                open_wrapper(&mut output, wrapper_tag, &mut wrapper_open);
                output.push_escaped_char(ch);
                prev_whitespace = false;
                atom_index += 1;
            }
        }
    }

    literalize_pending_stars(
        &mut output,
        wrapper_tag,
        &mut wrapper_open,
        &mut pending_open_strength,
    );

    if let Some(active_strength) = emphasis_strength {
        output.push_raw(em_tag_strength(active_strength as i32, true));
    }

    if wrapper_open && let Some(tag) = wrapper_tag {
        output.push_raw(&format!("</{tag}>"));
    }

    output.finish()
}

pub(super) fn render_inline_code_span(
    output: &mut MarkdownOutputBuilder,
    span: &super::inline_code::ParsedInlineCodeSpan,
) {
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

fn open_wrapper(
    output: &mut MarkdownOutputBuilder,
    wrapper_tag: Option<&str>,
    wrapper_open: &mut bool,
) {
    if *wrapper_open {
        return;
    }

    if let Some(tag) = wrapper_tag {
        output.push_raw(&format!("<{tag}>"));
        *wrapper_open = true;
    }
}

fn open_pending_emphasis(
    output: &mut MarkdownOutputBuilder,
    wrapper_tag: Option<&str>,
    wrapper_open: &mut bool,
    emphasis_strength: &mut Option<usize>,
    pending_open_strength: &mut usize,
) {
    if *pending_open_strength == 0 {
        return;
    }

    open_wrapper(output, wrapper_tag, wrapper_open);
    output.push_raw(em_tag_strength(*pending_open_strength as i32, false));
    *emphasis_strength = Some(*pending_open_strength);
    *pending_open_strength = 0;
}

fn literalize_pending_stars(
    output: &mut MarkdownOutputBuilder,
    wrapper_tag: Option<&str>,
    wrapper_open: &mut bool,
    pending_open_strength: &mut usize,
) {
    if *pending_open_strength == 0 {
        return;
    }

    open_wrapper(output, wrapper_tag, wrapper_open);
    output.push_raw(&"*".repeat(*pending_open_strength));
    *pending_open_strength = 0;
}

pub(super) fn count_consecutive_star_chars(
    atoms: &[MarkdownInlineAtom],
    start_index: usize,
) -> usize {
    let mut count = 0usize;
    let mut index = start_index;

    while let Some('*') = super::types::atom_char(atoms, index) {
        count += 1;
        index += 1;
    }

    count
}

pub(super) fn em_tag_strength(strength: i32, is_closing_tag: bool) -> &'static str {
    if is_closing_tag {
        match strength {
            2 => "</strong>",
            3 => "</strong></em>",
            _ => "</em>",
        }
    } else {
        match strength {
            2 => "<strong>",
            3 => "<em><strong>",
            _ => "<em>",
        }
    }
}

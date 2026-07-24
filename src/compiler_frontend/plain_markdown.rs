//! Plain Markdown renderer adapter.
//!
//! WHAT: renders raw plain Markdown text to HTML.
//!
//! WHY: `.md` files are plain content assets and must not enter Moth tokenization. This
//! module centralizes the small CommonMark/GFM-compatible rendering step so later stages can treat
//! Markdown as a generated `content #String` without duplicating parser configuration.
//!
//! MUST NOT: own import resolution, source-kind registration, diagnostics, asset emission, route
//! rewriting, Moth template behavior, or any compiler stage beyond raw-text-to-HTML conversion.

use pulldown_cmark::{Options, Parser, html};

pub(crate) struct RenderedPlainMarkdown {
    pub(crate) html: String,
}

pub(crate) fn render_plain_markdown(markdown: &str) -> RenderedPlainMarkdown {
    let options = commonmark_web_options();

    let mut html = String::with_capacity(markdown.len() + markdown.len() / 2);
    html::push_html(&mut html, Parser::new_ext(markdown, options));

    RenderedPlainMarkdown { html }
}

fn commonmark_web_options() -> Options {
    let mut options = Options::empty();

    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_GFM);

    options
}

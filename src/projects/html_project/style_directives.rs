//! HTML project style-directive registration.
//!
//! WHAT:
//! - Declares all non-core style directives used by the HTML build system.
//! - Provides full directive behavior through formatter factories and argument contracts.
//!
//! WHY:
//! - Non-core directive behavior should be owned by build systems, not hardcoded in frontend core.

use crate::compiler_frontend::style_directives::{
    StyleDirectiveArgumentType, StyleDirectiveEffects, StyleDirectiveHandlerSpec,
    StyleDirectiveSpec, TemplateHeadCompatibility, TemplateHeadConflictReason, TemplateHeadTag,
};
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;
use crate::projects::html_project::styles::code::code_formatter_factory;
use crate::projects::html_project::styles::css::css_formatter_factory;
use crate::projects::html_project::styles::escape_html::escape_html_formatter_factory;
use crate::projects::html_project::styles::html::html_formatter_factory;

/// Full project-owned style-directive set for the HTML project builder.
pub(crate) fn html_project_style_directives() -> Vec<StyleDirectiveSpec> {
    vec![
        StyleDirectiveSpec::handler(
            "html",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::FORMATTER_DIRECTIVE,
                TemplateHeadConflictReason::MultipleFormatters,
            ),
            StyleDirectiveHandlerSpec::new(
                None,
                StyleDirectiveEffects {
                    style_id: Some("html"),
                    ..StyleDirectiveEffects::default()
                },
                Some(html_formatter_factory),
            ),
        ),
        StyleDirectiveSpec::handler(
            "css",
            TemplateBodyMode::Balanced,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::FORMATTER_DIRECTIVE,
                TemplateHeadConflictReason::MultipleFormatters,
            ),
            StyleDirectiveHandlerSpec::new(
                Some(StyleDirectiveArgumentType::String),
                StyleDirectiveEffects {
                    style_id: Some("css"),
                    ..StyleDirectiveEffects::default()
                },
                Some(css_formatter_factory),
            ),
        ),
        StyleDirectiveSpec::handler(
            "escape_html",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::FORMATTER_DIRECTIVE,
                TemplateHeadConflictReason::MultipleFormatters,
            ),
            StyleDirectiveHandlerSpec::new(
                None,
                StyleDirectiveEffects {
                    style_id: Some("escape_html"),
                    ..StyleDirectiveEffects::default()
                },
                Some(escape_html_formatter_factory),
            ),
        ),
        StyleDirectiveSpec::handler(
            "code",
            TemplateBodyMode::Balanced,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::FORMATTER_DIRECTIVE
                    | TemplateHeadTag::CODE_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::empty(),
                blocks_future_tags: TemplateHeadTag::FORMATTER_DIRECTIVE,
                conflict_reason: Some(TemplateHeadConflictReason::MultipleFormatters),
            },
            StyleDirectiveHandlerSpec::new(
                Some(StyleDirectiveArgumentType::String),
                StyleDirectiveEffects {
                    style_id: Some("code"),
                    suppress_child_templates: Some(true),
                    ..StyleDirectiveEffects::default()
                },
                Some(code_formatter_factory),
            ),
        ),
    ]
}

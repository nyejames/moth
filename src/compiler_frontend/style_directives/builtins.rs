//! Frontend-owned style directive definitions.
//!
//! The order in this file is intentionally stable because diagnostic rendering lists supported
//! directives in registry order.

use crate::compiler_frontend::ast::templates::styles::code::code_formatter_factory;
use crate::compiler_frontend::ast::templates::styles::css::css_formatter_factory;
use crate::compiler_frontend::ast::templates::styles::escape_html::escape_html_formatter_factory;
use crate::compiler_frontend::ast::templates::styles::html::html_formatter_factory;
use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter_factory;
use crate::compiler_frontend::style_directives::TemplateHeadConflictReason;
use crate::compiler_frontend::style_directives::compatibility::{
    TemplateHeadCompatibility, TemplateHeadTag,
};
use crate::compiler_frontend::style_directives::specs::{
    CONFIG_DIRECTIVE_NAME, CoreStyleDirectiveKind, DOC_DIRECTIVE_NAME, EXPORT_DIRECTIVE_NAME,
    FAST_MATH_DIRECTIVE_NAME, INFALLIBLE_DIRECTIVE_NAME, INSERT_DIRECTIVE_NAME,
    SAFE_MATH_DIRECTIVE_NAME, SLOT_DIRECTIVE_NAME, SUPPLIED_DIRECTIVE_NAME, SourceDirectiveKind,
    SourceModifierOperand, StyleDirectiveArgumentType, StyleDirectiveHandlerSpec,
    StyleDirectiveSpec, TODO_DIRECTIVE_NAME,
};
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;

pub(crate) fn frontend_built_in_directives() -> Vec<StyleDirectiveSpec> {
    let formatter_compatibility = TemplateHeadCompatibility::blocks_same(
        TemplateHeadTag::FORMATTER_DIRECTIVE,
        TemplateHeadConflictReason::MultipleFormatters,
    );

    vec![
        StyleDirectiveSpec::core(
            "before",
            None,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::BEFORE_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::Before,
        ),
        StyleDirectiveSpec::core(
            "after",
            None,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::AFTER_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::After,
        ),
        StyleDirectiveSpec::core(
            "fresh",
            None,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::FRESH_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::Fresh,
        ),
        StyleDirectiveSpec::core(
            SLOT_DIRECTIVE_NAME,
            None,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM | TemplateHeadTag::SLOT_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                blocks_future_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                conflict_reason: Some(TemplateHeadConflictReason::SlotDirectiveMustBeExclusive),
            },
            CoreStyleDirectiveKind::Slot,
        ),
        StyleDirectiveSpec::core(
            SUPPLIED_DIRECTIVE_NAME,
            None,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::SUPPLIED_DIRECTIVE,
                // A guard selects its own body, so only formatter directives may share the head.
                required_absent_tags: TemplateHeadTag::EXPRESSION
                    | TemplateHeadTag::SLOT_DIRECTIVE
                    | TemplateHeadTag::IN_DIRECTIVE
                    | TemplateHeadTag::COMMENT_DIRECTIVE
                    | TemplateHeadTag::FRESH_DIRECTIVE
                    | TemplateHeadTag::BEFORE_DIRECTIVE
                    | TemplateHeadTag::AFTER_DIRECTIVE
                    | TemplateHeadTag::SUPPLIED_DIRECTIVE
                    | TemplateHeadTag::CONTROL_FLOW,
                blocks_future_tags: TemplateHeadTag::EXPRESSION
                    | TemplateHeadTag::SLOT_DIRECTIVE
                    | TemplateHeadTag::IN_DIRECTIVE
                    | TemplateHeadTag::COMMENT_DIRECTIVE
                    | TemplateHeadTag::FRESH_DIRECTIVE
                    | TemplateHeadTag::BEFORE_DIRECTIVE
                    | TemplateHeadTag::AFTER_DIRECTIVE
                    | TemplateHeadTag::SUPPLIED_DIRECTIVE
                    | TemplateHeadTag::CONTROL_FLOW,
                conflict_reason: Some(TemplateHeadConflictReason::SuppliedDirectiveMustBeExclusive),
            },
            CoreStyleDirectiveKind::Supplied,
        ),
        StyleDirectiveSpec::core(
            INSERT_DIRECTIVE_NAME,
            None,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::IN_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::In,
        ),
        StyleDirectiveSpec::core(
            "note",
            Some(TemplateBodyMode::DiscardBalanced),
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::COMMENT_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                blocks_future_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                conflict_reason: Some(
                    TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem,
                ),
            },
            CoreStyleDirectiveKind::Note,
        ),
        StyleDirectiveSpec::core(
            "raw",
            Some(TemplateBodyMode::Normal),
            formatter_compatibility,
            CoreStyleDirectiveKind::Raw,
        ),
        StyleDirectiveSpec::handler(
            "md",
            Some(TemplateBodyMode::Normal),
            formatter_compatibility,
            StyleDirectiveHandlerSpec {
                argument_type: None,
                style_id: "markdown",
                suppress_child_templates: false,
                formatter_factory: markdown_formatter_factory,
            },
        ),
        StyleDirectiveSpec::handler(
            "html",
            Some(TemplateBodyMode::Normal),
            formatter_compatibility,
            StyleDirectiveHandlerSpec {
                argument_type: None,
                style_id: "html",
                suppress_child_templates: false,
                formatter_factory: html_formatter_factory,
            },
        ),
        StyleDirectiveSpec::handler(
            "css",
            Some(TemplateBodyMode::Balanced),
            formatter_compatibility,
            StyleDirectiveHandlerSpec {
                argument_type: Some(StyleDirectiveArgumentType::String),
                style_id: "css",
                suppress_child_templates: false,
                formatter_factory: css_formatter_factory,
            },
        ),
        StyleDirectiveSpec::handler(
            "escape_html",
            Some(TemplateBodyMode::Normal),
            formatter_compatibility,
            StyleDirectiveHandlerSpec {
                argument_type: None,
                style_id: "escape_html",
                suppress_child_templates: false,
                formatter_factory: escape_html_formatter_factory,
            },
        ),
        StyleDirectiveSpec::handler(
            "code",
            Some(TemplateBodyMode::Balanced),
            formatter_compatibility,
            StyleDirectiveHandlerSpec {
                argument_type: Some(StyleDirectiveArgumentType::String),
                style_id: "code",
                suppress_child_templates: true,
                formatter_factory: code_formatter_factory,
            },
        ),
        StyleDirectiveSpec::source_modifier(
            EXPORT_DIRECTIVE_NAME,
            SourceDirectiveKind::Export,
            SourceModifierOperand::OptionalPublicRoute,
        ),
        StyleDirectiveSpec::source_modifier(
            CONFIG_DIRECTIVE_NAME,
            SourceDirectiveKind::Config,
            SourceModifierOperand::None,
        ),
        StyleDirectiveSpec::source_modifier(
            DOC_DIRECTIVE_NAME,
            SourceDirectiveKind::Doc,
            SourceModifierOperand::RequiredSingleString,
        ),
        StyleDirectiveSpec::source_modifier(
            TODO_DIRECTIVE_NAME,
            SourceDirectiveKind::Todo,
            SourceModifierOperand::RequiredSingleString,
        ),
        StyleDirectiveSpec::source_modifier(
            FAST_MATH_DIRECTIVE_NAME,
            SourceDirectiveKind::FastMath,
            SourceModifierOperand::None,
        ),
        StyleDirectiveSpec::source_modifier(
            SAFE_MATH_DIRECTIVE_NAME,
            SourceDirectiveKind::SafeMath,
            SourceModifierOperand::None,
        ),
        StyleDirectiveSpec::source_modifier(
            INFALLIBLE_DIRECTIVE_NAME,
            SourceDirectiveKind::Infallible,
            SourceModifierOperand::None,
        ),
    ]
}

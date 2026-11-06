//! Frontend-owned style directive definitions.
//!
//! The order in this file is intentionally stable because diagnostic rendering lists supported
//! directives in registry order.

use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter_factory;
use crate::compiler_frontend::style_directives::TemplateHeadConflictReason;
use crate::compiler_frontend::style_directives::compatibility::{
    TemplateHeadCompatibility, TemplateHeadTag,
};
use crate::compiler_frontend::style_directives::specs::{
    CoreStyleDirectiveKind, DOC_DIRECTIVE_NAME, INSERT_DIRECTIVE_NAME, SLOT_DIRECTIVE_NAME,
    StyleDirectiveEffects, StyleDirectiveHandlerSpec, StyleDirectiveSpec,
};
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;

pub(crate) fn frontend_built_in_directives() -> Vec<StyleDirectiveSpec> {
    vec![
        StyleDirectiveSpec::core(
            "before",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::BEFORE_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::Before,
        ),
        StyleDirectiveSpec::core(
            "after",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::AFTER_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::After,
        ),
        StyleDirectiveSpec::core(
            "fresh",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM | TemplateHeadTag::FRESH_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::empty(),
                blocks_future_tags: TemplateHeadTag::empty(),
                conflict_reason: None,
            },
            CoreStyleDirectiveKind::Fresh,
        ),
        StyleDirectiveSpec::core(
            SLOT_DIRECTIVE_NAME,
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM | TemplateHeadTag::SLOT_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                blocks_future_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                conflict_reason: Some(TemplateHeadConflictReason::SlotDirectiveMustBeExclusive),
            },
            CoreStyleDirectiveKind::Slot,
        ),
        StyleDirectiveSpec::core(
            INSERT_DIRECTIVE_NAME,
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility::blocks_same(
                TemplateHeadTag::IN_DIRECTIVE,
                TemplateHeadConflictReason::DuplicateExclusiveDirective,
            ),
            CoreStyleDirectiveKind::In,
        ),
        StyleDirectiveSpec::core(
            "note",
            TemplateBodyMode::DiscardBalanced,
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
            "todo",
            TemplateBodyMode::DiscardBalanced,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::COMMENT_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                blocks_future_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                conflict_reason: Some(
                    TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem,
                ),
            },
            CoreStyleDirectiveKind::Todo,
        ),
        StyleDirectiveSpec::core(
            DOC_DIRECTIVE_NAME,
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::COMMENT_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                blocks_future_tags: TemplateHeadTag::MEANINGFUL_ITEM,
                conflict_reason: Some(
                    TemplateHeadConflictReason::DocumentationDirectiveMustBeOnlyMeaningfulItem,
                ),
            },
            CoreStyleDirectiveKind::Doc,
        ),
        StyleDirectiveSpec::core(
            "raw",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::FORMATTER_DIRECTIVE
                    | TemplateHeadTag::RAW_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::empty(),
                blocks_future_tags: TemplateHeadTag::FORMATTER_DIRECTIVE,
                conflict_reason: Some(TemplateHeadConflictReason::MultipleFormatters),
            },
            CoreStyleDirectiveKind::Raw,
        ),
        StyleDirectiveSpec::handler(
            "md",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility {
                presence_tags: TemplateHeadTag::MEANINGFUL_ITEM
                    | TemplateHeadTag::FORMATTER_DIRECTIVE,
                required_absent_tags: TemplateHeadTag::empty(),
                blocks_future_tags: TemplateHeadTag::FORMATTER_DIRECTIVE,
                conflict_reason: Some(TemplateHeadConflictReason::MultipleFormatters),
            },
            StyleDirectiveHandlerSpec::new(
                None,
                StyleDirectiveEffects {
                    style_id: Some("markdown"),
                    ..StyleDirectiveEffects::default()
                },
                Some(markdown_formatter_factory),
            ),
        ),
    ]
}

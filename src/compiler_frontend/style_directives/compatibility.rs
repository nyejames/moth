//! Template-head compatibility tags for style directive parsing.
//!
//! Parser state needs cheap set membership checks while still keeping compatibility policy on
//! directive specs. Named bit tags make combinations clearer than threading many boolean flags
//! through tokenizer and template parsing code.

use crate::compiler_frontend::compiler_messages::TemplateHeadConflictReason;
use std::ops::{BitAnd, BitOr, BitOrAssign};

/// Template-head compatibility tags for directives and other meaningful head items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TemplateHeadTag(u64);

impl TemplateHeadTag {
    pub const MEANINGFUL_ITEM: Self = Self(1 << 0);
    pub const SLOT_DIRECTIVE: Self = Self(1 << 1);
    pub const IN_DIRECTIVE: Self = Self(1 << 2);
    pub const COMMENT_DIRECTIVE: Self = Self(1 << 3);
    pub const FORMATTER_DIRECTIVE: Self = Self(1 << 4);
    pub const BEFORE_DIRECTIVE: Self = Self(1 << 5);
    pub const FRESH_DIRECTIVE: Self = Self(1 << 6);
    pub const RAW_DIRECTIVE: Self = Self(1 << 7);
    pub const CODE_DIRECTIVE: Self = Self(1 << 8);
    pub const AFTER_DIRECTIVE: Self = Self(1 << 9);

    /// How many one-bit tag slots exist. Every declared tag constant uses one index below this.
    pub const SLOT_COUNT: usize = 10;

    pub const fn empty() -> Self {
        Self(0)
    }

    /// The one-bit tag indices this mask contains, in declaration order.
    pub fn indices(self) -> impl Iterator<Item = usize> {
        (0..Self::SLOT_COUNT).filter(move |index| (self.0 & (1 << index)) != 0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

// Per-tag conflict slots index by bit position, so every declared tag must sit below the
// slot count or a detected conflict could not name its earlier item.
const _: () = assert!(
    matches!(
        (TemplateHeadTag::MEANINGFUL_ITEM.0
            | TemplateHeadTag::SLOT_DIRECTIVE.0
            | TemplateHeadTag::IN_DIRECTIVE.0
            | TemplateHeadTag::COMMENT_DIRECTIVE.0
            | TemplateHeadTag::FORMATTER_DIRECTIVE.0
            | TemplateHeadTag::BEFORE_DIRECTIVE.0
            | TemplateHeadTag::AFTER_DIRECTIVE.0
            | TemplateHeadTag::FRESH_DIRECTIVE.0
            | TemplateHeadTag::RAW_DIRECTIVE.0
            | TemplateHeadTag::CODE_DIRECTIVE.0)
            .checked_shr(TemplateHeadTag::SLOT_COUNT as u32),
        Some(0)
    ),
    "raise TemplateHeadTag::SLOT_COUNT when adding a tag"
);

impl BitOr for TemplateHeadTag {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for TemplateHeadTag {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for TemplateHeadTag {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

/// Data-driven template-head compatibility rules attached to each directive spec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TemplateHeadCompatibility {
    pub presence_tags: TemplateHeadTag,
    pub required_absent_tags: TemplateHeadTag,
    pub blocks_future_tags: TemplateHeadTag,
    /// The semantic rule that rejects a conflicting pair. A specification whose tags can reject
    /// another item must declare one so the diagnostic can name both items and explain the rule.
    pub conflict_reason: Option<TemplateHeadConflictReason>,
}

impl TemplateHeadCompatibility {
    pub fn fully_compatible_meaningful() -> Self {
        Self {
            presence_tags: TemplateHeadTag::MEANINGFUL_ITEM,
            required_absent_tags: TemplateHeadTag::empty(),
            blocks_future_tags: TemplateHeadTag::empty(),
            conflict_reason: None,
        }
    }

    pub fn exclusive_meaningful(conflict_reason: TemplateHeadConflictReason) -> Self {
        Self {
            presence_tags: TemplateHeadTag::MEANINGFUL_ITEM,
            required_absent_tags: TemplateHeadTag::MEANINGFUL_ITEM,
            blocks_future_tags: TemplateHeadTag::MEANINGFUL_ITEM,
            conflict_reason: Some(conflict_reason),
        }
    }

    pub fn blocks_same(tag: TemplateHeadTag, conflict_reason: TemplateHeadConflictReason) -> Self {
        Self {
            presence_tags: TemplateHeadTag::MEANINGFUL_ITEM | tag,
            required_absent_tags: TemplateHeadTag::empty(),
            blocks_future_tags: tag,
            conflict_reason: Some(conflict_reason),
        }
    }
}

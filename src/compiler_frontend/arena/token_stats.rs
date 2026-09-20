//! Cheap per-file token classification for arena capacity estimates.
//!
//! WHAT: counts simple TokenTag categories while tokenization already produces tokens.
//! WHY: these counts are policy-only seeds for capacity heuristics; they never affect
//!      diagnostics, ordering, lowering, type identity, or emitted artifacts.

use crate::compiler_frontend::tokenizer::tokens::{TokenShape, TokenTag};

/// Cheap token counts gathered during lexing.
///
/// WHAT: a small, Copy-able snapshot of token volume by broad category. It carries no interned
///      string IDs, so it needs no string-table remap when per-file outputs merge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TokenStats {
    pub total_tokens: usize,
    pub symbols: usize,
    pub literals: usize,
    pub operators: usize,
    pub template_markers: usize,
    pub style_directives: usize,
    pub hashes: usize,
    pub if_tokens: usize,
    pub loop_tokens: usize,
    pub catch_tokens: usize,
    pub then_tokens: usize,
    pub return_tokens: usize,
    pub cast_tokens: usize,
    pub mutable_markers: usize,
    pub map_or_collection_delimiters: usize,
}

impl TokenStats {
    /// Update all category counters for one canonical source token.
    ///
    /// WHAT: classifies a single canonical `TokenShape` into the cheap buckets used for
    ///      capacity estimates. `TokenTag` (and its schema authority) is the single
    ///      classification source; no second hand-maintained operator table lives here.
    /// WHY: called once per token while source-token construction already packs shapes,
    ///      avoiding both a separate full-token traversal and a second classification pass.
    pub(crate) fn accumulate_shape(&mut self, shape: TokenShape) {
        self.accumulate_tag(shape.tag());
    }

    /// Update all category counters for one stable token tag.
    ///
    /// WHAT: the tag-level classification behind every `TokenStats` entry point, so shape
    ///      and tag inputs share one bucket decision.
    /// WHY: keeps the schema as the single classification authority for capacity seeds.
    pub(crate) fn accumulate_tag(&mut self, tag: TokenTag) {
        self.total_tokens += 1;

        if tag.is_stats_symbol() {
            self.symbols += 1;
            return;
        }

        if tag.is_stats_literal() {
            self.literals += 1;
            return;
        }

        if tag.is_stats_operator() {
            self.operators += 1;
            return;
        }

        match tag {
            TokenTag::TEMPLATE_HEAD | TokenTag::TEMPLATE_CLOSE | TokenTag::START_TEMPLATE_BODY => {
                self.template_markers += 1;
            }

            TokenTag::STYLE_DIRECTIVE => {
                self.style_directives += 1;
            }

            TokenTag::HASH => {
                self.hashes += 1;
            }

            TokenTag::IF => {
                self.if_tokens += 1;
            }

            TokenTag::LOOP => {
                self.loop_tokens += 1;
            }

            TokenTag::CATCH => {
                self.catch_tokens += 1;
            }

            TokenTag::THEN => {
                self.then_tokens += 1;
            }

            TokenTag::RETURN | TokenTag::RETURN_BANG => {
                self.return_tokens += 1;
            }

            TokenTag::CAST | TokenTag::CAST_BANG => {
                self.cast_tokens += 1;
            }

            TokenTag::MUTABLE => {
                self.mutable_markers += 1;
            }

            TokenTag::OPEN_CURLY | TokenTag::CLOSE_CURLY | TokenTag::COMMA => {
                self.map_or_collection_delimiters += 1;
            }

            _ => {}
        }
    }

    /// Merge another per-file snapshot into this one.
    ///
    /// WHAT: adds each bucket, producing a module-wide aggregate.
    /// WHY: per-file stats are merged deterministically after parallel preparation finishes.
    pub(crate) fn add(&mut self, other: &TokenStats) {
        self.total_tokens += other.total_tokens;
        self.symbols += other.symbols;
        self.literals += other.literals;
        self.operators += other.operators;
        self.template_markers += other.template_markers;
        self.style_directives += other.style_directives;
        self.hashes += other.hashes;
        self.if_tokens += other.if_tokens;
        self.loop_tokens += other.loop_tokens;
        self.catch_tokens += other.catch_tokens;
        self.then_tokens += other.then_tokens;
        self.return_tokens += other.return_tokens;
        self.cast_tokens += other.cast_tokens;
        self.mutable_markers += other.mutable_markers;
        self.map_or_collection_delimiters += other.map_or_collection_delimiters;
    }
}

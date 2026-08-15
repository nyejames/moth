//! Cheap per-file token classification for arena capacity estimates.
//!
//! WHAT: counts simple token-kind categories while tokenization already produces tokens.
//! WHY: these counts are policy-only seeds for capacity heuristics; they never affect
//!      diagnostics, ordering, lowering, type identity, or emitted artifacts.

use crate::compiler_frontend::tokenizer::tokens::TokenKind;

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
    /// Update all category counters for one emitted token.
    ///
    /// WHAT: classifies a single `TokenKind` into the cheap buckets used for capacity estimates.
    /// WHY: called once per token during the existing tokenization loop, avoiding a separate
    ///      full-token traversal.
    pub(crate) fn accumulate(&mut self, kind: &TokenKind) {
        self.total_tokens += 1;

        match kind {
            TokenKind::Symbol(_) => {
                self.symbols += 1;
            }

            TokenKind::StringSliceLiteral(_)
            | TokenKind::RawStringLiteral(_)
            | TokenKind::NumericLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::NoneLiteral => {
                self.literals += 1;
            }

            TokenKind::Add
            | TokenKind::Subtract
            | TokenKind::Multiply
            | TokenKind::Divide
            | TokenKind::Modulus
            | TokenKind::IntDivide
            | TokenKind::Exponent
            | TokenKind::Negative
            | TokenKind::AddAssign
            | TokenKind::SubtractAssign
            | TokenKind::MultiplyAssign
            | TokenKind::DivideAssign
            | TokenKind::ModulusAssign
            | TokenKind::ExponentAssign
            | TokenKind::IntDivideAssign
            | TokenKind::LessThan
            | TokenKind::LessThanOrEqual
            | TokenKind::GreaterThan
            | TokenKind::GreaterThanOrEqual
            | TokenKind::Is
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::Bang
            | TokenKind::QuestionMark
            | TokenKind::Copy
            | TokenKind::ChannelSend
            | TokenKind::ChannelReceive
            | TokenKind::Ampersand
            | TokenKind::Arrow
            | TokenKind::FatArrow => {
                self.operators += 1;
            }

            TokenKind::TemplateHead | TokenKind::TemplateClose | TokenKind::StartTemplateBody => {
                self.template_markers += 1;
            }

            TokenKind::StyleDirective(_) => {
                self.style_directives += 1;
            }

            TokenKind::Hash => {
                self.hashes += 1;
            }

            TokenKind::If => {
                self.if_tokens += 1;
            }

            TokenKind::Loop => {
                self.loop_tokens += 1;
            }

            TokenKind::Catch => {
                self.catch_tokens += 1;
            }

            TokenKind::Then => {
                self.then_tokens += 1;
            }

            TokenKind::Return | TokenKind::ReturnBang => {
                self.return_tokens += 1;
            }

            TokenKind::Cast | TokenKind::CastBang => {
                self.cast_tokens += 1;
            }

            TokenKind::Mutable => {
                self.mutable_markers += 1;
            }

            TokenKind::OpenCurly | TokenKind::CloseCurly | TokenKind::Comma => {
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

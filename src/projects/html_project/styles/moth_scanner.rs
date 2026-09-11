//! Moth-specific scanner state and lexical extensions.
//!
//! The parent `code` module owns the shared scanner shell, role vocabulary and
//! span emission. This module owns the Moth contextual state machines and the
//! tokenization branches whose behaviour is specific to Moth, while also
//! housing the shared number/operator routines that dispatch into them.

use super::{
    CodeHighlightRole, CodeLanguage, CodeScanner, language_profiles::NON_MOTH_OPERATOR_BYTES,
};
use crate::compiler_frontend::builtins::error_type::ERROR_TYPE_NAME;
use crate::compiler_frontend::external_packages::IO_NAMESPACE_NAME;
use crate::compiler_frontend::keywords::{
    SourceWordClass, attached_bang_keyword_token_kind, classify_source_word,
};
use crate::compiler_frontend::symbols::identifier_policy::is_uppercase_constant_name;

/// Contract-list kind for the Moth heuristic.
///
/// WHAT: distinguishes trait conformance lists (`must`, `must not`) from
///       generic-bound lists after `is` inside a generic declaration.
/// WHY: commas continue conformance lists but end a generic bound list so the
///      next identifier can be a new generic parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContractListKind {
    Conformance,
    GenericBound,
}

/// Bounded contract-list state for the Moth heuristic.
///
/// WHAT: remembers whether the next uppercase-constant identifier is a
///       contract name and which kind of list expects it. An expectation
///       armed by a conformance comma survives a newline; every other
///       expectation dies at declaration boundaries.
/// WHY: casing alone must never decide the Contract role, so the scanner
///      needs a tiny expectation that structural boundaries reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContractState {
    None,
    ExpectName {
        kind: ContractListKind,
        continued_after_comma: bool,
    },
    AfterName(ContractListKind),
}

/// Bounded presentation state for one delimiter-free source dependency clause.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DependencyHighlightState {
    #[default]
    None,
    AfterPath,
    ExpectNamespaceAlias,
    AfterNamespaceAlias,
    AfterSelection,
    ExpectSelectionAlias,
    AfterSelectionAlias,
    ExpectSelection,
}

impl<'source> CodeScanner<'source> {
    /// Returns the byte index just past one identifier word.
    ///
    /// WHAT: scans ASCII alphanumerics and underscores, consumes one scalar for
    ///       any non-ASCII byte, then includes an attached `return!` / `cast!`
    ///       bang for Moth.
    /// WHY: the word range is computed locally so the shared emitter can hold the
    ///      single invariant that the cursor is still at the token start.
    pub(super) fn word_end(&self, start: usize) -> usize {
        let mut end = start;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii() {
                if byte.is_ascii_alphanumeric() || byte == b'_' {
                    end += 1;
                } else {
                    break;
                }
            } else {
                let ch = self.source[end..]
                    .chars()
                    .next()
                    .expect("scan position is always on a char boundary");
                end += ch.len_utf8();
            }
        }

        // `return!` and `cast!` keep the attached bang inside the keyword span.
        if self.language == CodeLanguage::Moth
            && end < self.bytes.len()
            && self.bytes[end] == b'!'
            && attached_bang_keyword_token_kind(&self.source[start..end]).is_some()
        {
            end += 1;
        }

        end
    }

    /// Classifies one Moth word through the compiler-owned classes and the
    /// bounded lexical heuristics.
    pub(super) fn moth_word_role(
        &mut self,
        word: &str,
        word_end: usize,
    ) -> Option<CodeHighlightRole> {
        if let Some(role) = self.dependency_word_role(word) {
            return Some(role);
        }

        // Attached bang forms are keyword spans.
        if let Some(prefix) = word.strip_suffix('!')
            && attached_bang_keyword_token_kind(prefix).is_some()
        {
            self.reset_declaration_context();
            return Some(CodeHighlightRole::Keyword);
        }

        if let Some(classified) = classify_source_word(word) {
            let role = match classified.class {
                SourceWordClass::Keyword => CodeHighlightRole::Keyword,
                SourceWordClass::WordOperator => CodeHighlightRole::Operator,
                SourceWordClass::Literal => CodeHighlightRole::Literal,
                SourceWordClass::BuiltinType => CodeHighlightRole::Type,
            };

            // Word-level contract-list transitions.
            match word {
                "type" => {
                    self.generic_declaration = true;
                    self.contract_state = ContractState::None;
                }
                "loop" => {
                    // A collection or range loop keeps its source/projection
                    // unclassified until the header ends at `:` or a newline.
                    self.loop_header_depth = Some(self.moth_delimiter_depth);
                    self.contract_state = ContractState::None;
                }
                "must" => {
                    self.generic_declaration = false;
                    self.contract_state = ContractState::ExpectName {
                        kind: ContractListKind::Conformance,
                        continued_after_comma: false,
                    };
                }
                "not"
                    if matches!(
                        self.contract_state,
                        ContractState::ExpectName {
                            kind: ContractListKind::Conformance,
                            ..
                        }
                    ) => {}
                "is" => {
                    self.contract_state = if self.generic_declaration {
                        ContractState::ExpectName {
                            kind: ContractListKind::GenericBound,
                            continued_after_comma: false,
                        }
                    } else {
                        ContractState::None
                    };
                }
                "and" if matches!(self.contract_state, ContractState::AfterName(_)) => {
                    let ContractState::AfterName(kind) = self.contract_state else {
                        unreachable!("guarded by the match arm above");
                    };
                    self.contract_state = ContractState::ExpectName {
                        kind,
                        continued_after_comma: false,
                    };
                }
                _ => self.contract_state = ContractState::None,
            }

            return Some(role);
        }

        // Canonical builtin spellings that are not tokenizer keywords.
        if word == ERROR_TYPE_NAME {
            self.contract_state = ContractState::None;
            return Some(CodeHighlightRole::Type);
        }

        if word == IO_NAMESPACE_NAME && self.bytes.get(word_end) == Some(&b'.') {
            self.contract_state = ContractState::None;
            return Some(CodeHighlightRole::Type);
        }

        // Contract names follow the compiler's uppercase-constant policy so
        // single letters, digits and underscores classify exactly as traits do
        // in Moth. The policy applies only in contract context; ordinary `A`,
        // `E` and digit-bearing constants keep their nominal/plain fallback.
        if is_uppercase_constant_name(word) {
            let in_contract_context = self.contract_state != ContractState::None
                || self.all_caps_followed_by_must(word_end);
            if in_contract_context {
                let kind = match self.contract_state {
                    ContractState::ExpectName { kind, .. } | ContractState::AfterName(kind) => kind,
                    // An uppercase-constant name followed by `must` declares a trait.
                    ContractState::None => ContractListKind::Conformance,
                };
                self.contract_state = ContractState::AfterName(kind);

                return Some(CodeHighlightRole::Contract);
            }

            // Outside contract context the compiler policy does not apply:
            // `A` and `E` keep their nominal fallback below, while all-caps
            // constants such as `PI` and `MAX_SIZE` stay plain.
            self.contract_state = ContractState::None;
        }

        if is_pascal_case_word(word) {
            self.contract_state = ContractState::None;
            return Some(CodeHighlightRole::Nominal);
        }

        // Ordinary identifiers become functions before `(`, before a pipe that
        // opens a new group, or when they own a generic declaration
        // (`name type T ...`). Loop sources and projections are not
        // declarations even though a binding pipe follows, and identifiers
        // inside `|...|` stay plain.
        self.contract_state = ContractState::None;
        match self.next_non_horizontal_whitespace_byte(word_end) {
            Some(b'(') => Some(CodeHighlightRole::Function),
            Some(b'|') if !self.in_pipe_group && self.loop_header_depth.is_none() => {
                Some(CodeHighlightRole::Function)
            }
            _ if !self.in_pipe_group && self.next_word_is(word_end, "type") => {
                Some(CodeHighlightRole::Function)
            }
            _ => None,
        }
    }

    /// Resets contract-list and generic-declaration context at structural
    /// boundaries so later source cannot inherit stale expectations.
    ///
    /// Loop-header state is deliberately separate: it must survive nested
    /// delimiters and source-expression operators and ends only at its own
    /// top-level `|`, header `:` or terminating newline.
    pub(super) fn reset_declaration_context(&mut self) {
        self.contract_state = ContractState::None;
        self.generic_declaration = false;
    }

    /// Tracks Moth delimiter nesting for the loop-header heuristic.
    pub(super) fn update_moth_delimiter_depth(&mut self) {
        match self.bytes[self.index] {
            b'(' | b'[' | b'{' => self.moth_delimiter_depth += 1,
            b')' | b']' | b'}' => {
                self.moth_delimiter_depth = self.moth_delimiter_depth.saturating_sub(1)
            }
            _ => {}
        }
    }

    /// Ends loop-header context at a pipe or colon at the same nesting depth
    /// as the `loop` keyword.
    ///
    /// WHY: the binding pipe and the header colon are top-level boundaries;
    ///      the same byte inside a nested source expression must not end the
    ///      header early.
    pub(super) fn end_loop_header_at_top_level(&mut self) {
        if self.loop_header_depth == Some(self.moth_delimiter_depth) {
            self.loop_header_depth = None;
        }
    }

    /// Resets declaration context at a newline, preserving only a conformance
    /// continuation that a comma explicitly armed.
    ///
    /// WHY: `Label must FIRST,\n SECOND` stays one conformance list, while an
    ///      ordinary newline ends every declaration expectation.
    pub(super) fn reset_after_newline(&mut self) {
        self.loop_header_depth = None;
        self.generic_declaration = false;

        if !matches!(
            self.contract_state,
            ContractState::ExpectName {
                kind: ContractListKind::Conformance,
                continued_after_comma: true,
            }
        ) {
            self.contract_state = ContractState::None;
        }

        if self.dependency_state != DependencyHighlightState::ExpectSelection {
            self.dependency_state = DependencyHighlightState::None;
        }
    }

    /// Continues a conformance list after a comma or ends a generic bound list
    /// so the next word can be a new generic parameter.
    pub(super) fn transition_after_comma(&mut self) {
        self.contract_state = match self.contract_state {
            ContractState::AfterName(ContractListKind::Conformance) => ContractState::ExpectName {
                kind: ContractListKind::Conformance,
                continued_after_comma: true,
            },
            ContractState::AfterName(ContractListKind::GenericBound) => ContractState::None,
            _ => self.contract_state,
        };
    }

    fn dependency_word_role(&mut self, word: &str) -> Option<CodeHighlightRole> {
        match self.dependency_state {
            DependencyHighlightState::AfterPath if word == "as" => {
                self.dependency_state = DependencyHighlightState::ExpectNamespaceAlias;
                Some(CodeHighlightRole::Keyword)
            }
            DependencyHighlightState::AfterPath | DependencyHighlightState::ExpectSelection => {
                self.dependency_state = DependencyHighlightState::AfterSelection;
                Some(CodeHighlightRole::Nominal)
            }
            DependencyHighlightState::AfterSelection if word == "as" => {
                self.dependency_state = DependencyHighlightState::ExpectSelectionAlias;
                Some(CodeHighlightRole::Keyword)
            }
            DependencyHighlightState::ExpectNamespaceAlias => {
                self.dependency_state = DependencyHighlightState::AfterNamespaceAlias;
                Some(CodeHighlightRole::Nominal)
            }
            DependencyHighlightState::ExpectSelectionAlias => {
                self.dependency_state = DependencyHighlightState::AfterSelectionAlias;
                Some(CodeHighlightRole::Nominal)
            }
            DependencyHighlightState::AfterNamespaceAlias
            | DependencyHighlightState::AfterSelection
            | DependencyHighlightState::AfterSelectionAlias
            | DependencyHighlightState::None => {
                self.dependency_state = DependencyHighlightState::None;
                None
            }
        }
    }

    pub(super) fn continue_dependency_after_comma(&mut self) {
        self.dependency_state = match self.dependency_state {
            DependencyHighlightState::AfterSelection
            | DependencyHighlightState::AfterSelectionAlias => {
                DependencyHighlightState::ExpectSelection
            }
            _ => DependencyHighlightState::None,
        };
    }

    /// True when the next word after horizontal whitespace is exactly
    /// `expected`.
    ///
    /// WHY: a generic function owner is recognised by its `name type T ...`
    ///      shape, but only when `type` is the immediate next word.
    fn next_word_is(&self, from: usize, expected: &str) -> bool {
        let mut index = from;
        while index < self.bytes.len() && matches!(self.bytes[index], b' ' | b'\t') {
            index += 1;
        }

        let end = self.word_end(index);
        end > index && self.source[index..end] == *expected
    }

    pub(super) fn scan_moth_directive(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                end += 1;
            } else {
                break;
            }
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Directive);
    }

    pub(super) fn scan_moth_path(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len() && is_moth_path_byte(self.bytes[end]) {
            end += 1;
        }

        self.expected_word_role = None;
        self.dependency_state = if self.moth_dependency_clause_path_starts_here(run_start) {
            DependencyHighlightState::AfterPath
        } else {
            DependencyHighlightState::None
        };
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::String);
    }

    fn moth_dependency_clause_path_starts_here(&self, path_start: usize) -> bool {
        let line_start = self.source[..path_start]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        let prefix = self.source[line_start..path_start].trim();
        prefix.is_empty() || prefix == "export:"
    }

    /// Recognises Moth numeric runs: digits with separators, a decimal fraction
    /// and a lowercase exponent with an optional sign.
    ///
    /// WHY: tolerant by design. Range, separator placement and finiteness are
    ///      validated by the real tokenizer, never by the presentation scanner.
    fn moth_number_end(&self) -> usize {
        let mut end = consume_while(self.bytes, self.index, |byte| {
            byte.is_ascii_digit() || byte == b'_'
        });

        if end + 1 < self.bytes.len()
            && self.bytes[end] == b'.'
            && self.bytes[end + 1].is_ascii_digit()
        {
            end = consume_while(self.bytes, end + 1, |byte| {
                byte.is_ascii_digit() || byte == b'_'
            });
        }

        if end < self.bytes.len() && self.bytes[end] == b'e' {
            let mut exponent_end = end + 1;
            if exponent_end < self.bytes.len()
                && (self.bytes[exponent_end] == b'+' || self.bytes[exponent_end] == b'-')
            {
                exponent_end += 1;
            }

            if exponent_end < self.bytes.len() && self.bytes[exponent_end].is_ascii_digit() {
                end = consume_while(self.bytes, exponent_end, |byte| {
                    byte.is_ascii_digit() || byte == b'_'
                });
            }
        }

        end
    }

    pub(super) fn scan_number(&mut self, output: &mut String) {
        let run_start = self.index;
        let end = match self.language {
            CodeLanguage::Moth => self.moth_number_end(),
            _ => self.legacy_number_end(),
        };

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Number);
    }

    /// Preserves the pre-scanner numeric run for non-Moth profiles: digits,
    /// underscores, Unicode numeric scalars and a decimal point that is
    /// followed by a digit.
    ///
    /// WHY: allowing every dot would swallow Rust range operators (`0..10`)
    ///      and float method access (`1.0.to_string()`), so a dot only joins
    ///      the run when it starts a fractional part.
    fn legacy_number_end(&self) -> usize {
        let mut end = self.index;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii() {
                if byte.is_ascii_digit() || byte == b'_' {
                    end += 1;
                    continue;
                }
                if byte == b'.'
                    && self
                        .bytes
                        .get(end + 1)
                        .is_some_and(|next| next.is_ascii_digit())
                {
                    end += 1;
                    continue;
                }
                break;
            }

            let ch = self.source[end..]
                .chars()
                .next()
                .expect("number position is on a char boundary");
            if ch.is_numeric() {
                end += ch.len_utf8();
            } else {
                break;
            }
        }

        end
    }

    pub(super) fn scan_operator(&mut self, output: &mut String) {
        let Some(length) = self.operator_length() else {
            self.index += 1;
            return;
        };

        let run_start = self.index;
        let end = run_start + length;

        // Moth operators are structural boundaries for the contract heuristic,
        // including `=`, `->`, `<=` and `>=`, which never continue a contract list.
        if self.language == CodeLanguage::Moth {
            self.reset_declaration_context();
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Operator);
    }

    pub(super) fn operator_length(&self) -> Option<usize> {
        match self.language {
            CodeLanguage::Moth => self.moth_operator_length(),
            _ => {
                if NON_MOTH_OPERATOR_BYTES.contains(&self.bytes[self.index]) {
                    Some(1)
                } else {
                    None
                }
            }
        }
    }

    /// Returns the maximal-munch length of one Moth operator.
    ///
    /// WHAT: checks longer compound forms before their prefixes so `//=` is one
    ///       span and `::`, `..`, `->` and `=>` stay whole tokens.
    /// WHY: `==`, `!=` and `&&` are not Moth operators, so the fallback table
    ///      never combines those byte pairs into one invented token.
    fn moth_operator_length(&self) -> Option<usize> {
        let bytes = &self.bytes[self.index..];
        let next = |offset: usize| bytes.get(offset).copied();

        match bytes[0] {
            b'/' => {
                if next(1) == Some(b'/') {
                    if next(2) == Some(b'=') {
                        return Some(3);
                    }
                    return Some(2);
                }
                if next(1) == Some(b'=') {
                    return Some(2);
                }
                Some(1)
            }
            b'+' | b'*' | b'%' | b'^' | b'#' | b'~' | b'$' => {
                if next(1) == Some(b'=') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'-' => {
                if next(1) == Some(b'=') || next(1) == Some(b'>') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'=' => {
                if next(1) == Some(b'>') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'<' => {
                if next(1) == Some(b'<') || next(1) == Some(b'=') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'>' => {
                if next(1) == Some(b'>') || next(1) == Some(b'=') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b':' => {
                if next(1) == Some(b':') {
                    Some(2)
                } else {
                    None
                }
            }
            b'.' => {
                if next(1) == Some(b'.') {
                    Some(2)
                } else {
                    None
                }
            }
            b'!' | b'?' | b'&' | b'@' => Some(1),
            _ => None,
        }
    }

    pub(super) fn moth_directive_starts_here(&self) -> bool {
        matches!(
            self.bytes.get(self.index + 1),
            Some(b'a'..=b'z') | Some(b'_')
        )
    }

    pub(super) fn moth_path_starts_here(&self) -> bool {
        if !matches!(self.bytes.get(self.index + 1), Some(byte) if is_moth_path_byte(*byte)) {
            return false;
        }

        // A path may start only at a lexical boundary: not directly after
        // another `@` and not after a path or identifier continuation byte,
        // including a Unicode identifier continuation. This keeps invalid
        // doubled prefixes such as `@@name` and attached forms such as
        // `π@core/io` visible as plain source.
        self.index == 0 || !self.previous_scalar_continues_path_or_word()
    }

    /// True when the scalar immediately before the cursor is a path or
    /// identifier continuation.
    ///
    /// WHY: ASCII continuation bytes are checked directly, while a non-ASCII
    ///      previous byte must be decoded so a Unicode identifier such as `π`
    ///      blocks an attached `@path`.
    fn previous_scalar_continues_path_or_word(&self) -> bool {
        let before = self.index - 1;
        let byte = self.bytes[before];

        if byte.is_ascii() {
            return byte == b'@' || is_moth_path_byte(byte);
        }

        // Walk back from the previous byte over UTF-8 continuation bytes to
        // the scalar's leading byte, then reject when that scalar continues
        // an identifier.
        let mut scalar_start = before;
        while scalar_start > 0 && self.bytes[scalar_start] & 0xC0 == 0x80 {
            scalar_start -= 1;
        }

        self.source[scalar_start..self.index]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    }

    fn all_caps_followed_by_must(&self, word_end: usize) -> bool {
        let mut index = word_end;
        while index < self.bytes.len() && (self.bytes[index] == b' ' || self.bytes[index] == b'\t')
        {
            index += 1;
        }

        if !self.bytes[index..].starts_with(b"must") {
            return false;
        }

        !matches!(
            self.bytes.get(index + 4),
            None | Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'0'..=b'9') | Some(b'_')
        )
    }
}

/// Advances `index` while the byte predicate holds.
fn consume_while(bytes: &[u8], mut index: usize, predicate: impl Fn(u8) -> bool) -> usize {
    while index < bytes.len() && predicate(bytes[index]) {
        index += 1;
    }
    index
}

/// True when `byte` may continue a tolerant Moth dependency/resource path run.
fn is_moth_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'/' | b'.' | b'-')
}

/// True for ALL_CAPS identifiers with at least two letters.
///
/// WHY: single uppercase letters act as generic parameter names and stay
///      nominal, while `PI`, `TAU` and `DISPLAY_TEXT` use the all-caps shape.
///      This is the presentation split for the nominal fallback; contract
///      eligibility reuses the compiler's `is_uppercase_constant_name`
///      policy, so `A`, `TRAIT2` and `HTTP_2` classify as traits in contract
///      context.
fn is_all_caps_word(word: &str) -> bool {
    let mut letter_count = 0usize;

    for ch in word.chars() {
        if ch == '_' {
            continue;
        }
        if !ch.is_uppercase() {
            return false;
        }
        letter_count += 1;
    }

    letter_count >= 2
}

/// True for PascalCase identifiers that are not all-caps.
fn is_pascal_case_word(word: &str) -> bool {
    word.chars().next().is_some_and(|ch| ch.is_uppercase()) && !is_all_caps_word(word)
}

//! Compiler-owned source file kind recognition and builder-selected optional kinds.
//!
//! WHAT: classifies source extensions by kind and tracks which optional kinds the active builder
//! supports.
//! WHY: extension recognition belongs to the compiler so `.moth` and `.mtf` behave identically
//!      under every builder. Only kinds whose availability is a builder decision are registrable.

use crate::projects::settings::{
    CONTENT_EXTENSION, CONTENT_SUFFIX, LANGUAGE_SOURCE_EXTENSION, LANGUAGE_SOURCE_SUFFIX,
    MARKDOWN_EXTENSION, MARKDOWN_SUFFIX,
};
use std::collections::BTreeSet;

/// Identifies a category of source file that the compiler can ingest.
///
/// WHAT: distinguishes compiler-supported language and template source from builder-selected
///       content kinds.
/// WHY: Stage 0 discovery and later frontend stages branch on source kind to apply the
///      correct tokenization, header preparation, and AST lowering rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceFileKind {
    /// Standard language source files.
    Moth,
    /// Template content files.
    MothTemplate,
    /// Plain Markdown content files.
    ///
    /// WHY: HTML projects can bind Markdown as a generated `content #String` constant.
    PlainMarkdown,
}

/// A source kind whose support is the active builder's decision.
///
/// WHY: `.moth` and `.mtf` are compiler-supported under every builder, so a builder cannot
///      disable, reassign or substitute them. Limiting registration to this type keeps that
///      policy in the type system instead of a rejected runtime registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptionalSourceKind {
    /// Plain Markdown content files, bound as a generated `content #String` constant.
    PlainMarkdown,
}

/// A single compiler-recognized source file kind mapping.
///
/// WHAT: pairs a file extension with its source kind for lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupportedSourceFileKind {
    pub extension: &'static str,
    pub kind: SourceFileKind,
}

/// Registry of builder-selected optional source kinds.
///
/// WHAT: collects the optional kinds the active builder supports.
/// WHY: source-kind support is declarative and builder-local, but only for kinds the compiler
///      does not already support everywhere.
#[derive(Clone, Debug, Default)]
pub struct SourceFileKindRegistry {
    optional_kinds: BTreeSet<OptionalSourceKind>,
}

impl SourceFileKindRegistry {
    /// Creates an empty registry. Compiler-supported kinds still resolve and need no entry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Supports one optional source kind under this builder.
    pub fn register(&mut self, kind: OptionalSourceKind) {
        self.optional_kinds.insert(kind);
    }

    /// Looks up the supported source kind for a file extension.
    ///
    /// Returns `None` for unrecognized extensions and for optional kinds the active builder did
    /// not select. Compiler-supported kinds are always returned.
    pub fn kind_for_extension(&self, extension: &str) -> Option<SourceFileKind> {
        let kind = SourceFileKind::from_extension(extension)?;
        match kind.optional() {
            Some(optional) if !self.optional_kinds.contains(&optional) => None,
            _ => Some(kind),
        }
    }
}

impl SourceFileKind {
    /// Looks up compiler-recognized source-file kinds by extension.
    ///
    /// WHAT: separates recognition, which is compiler-owned, from active-builder support.
    /// WHY: Stage 0 must diagnose a known but unsupported optional kind, such as plain Markdown
    ///      under a builder that did not select it, instead of falling through to a
    ///      missing-dependency error.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            LANGUAGE_SOURCE_EXTENSION => Some(Self::Moth),
            CONTENT_EXTENSION => Some(Self::MothTemplate),
            MARKDOWN_EXTENSION => Some(Self::PlainMarkdown),
            _ => None,
        }
    }

    /// Returns the builder-selectable kind, or `None` when every builder supports this kind.
    ///
    /// `.moth` and `.mtf` are fixed compiler-supported kinds. This mapping is the only place that
    /// decides which kinds a builder may select.
    pub const fn optional(self) -> Option<OptionalSourceKind> {
        match self {
            Self::Moth | Self::MothTemplate => None,
            Self::PlainMarkdown => Some(OptionalSourceKind::PlainMarkdown),
        }
    }

    /// Returns the canonical extension for this source-file kind.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Moth => LANGUAGE_SOURCE_EXTENSION,
            Self::MothTemplate => CONTENT_EXTENSION,
            Self::PlainMarkdown => MARKDOWN_EXTENSION,
        }
    }

    /// Returns the canonical extension suffix used in source symbol paths.
    pub fn extension_suffix(self) -> &'static str {
        match self {
            Self::Moth => LANGUAGE_SOURCE_SUFFIX,
            Self::MothTemplate => CONTENT_SUFFIX,
            Self::PlainMarkdown => MARKDOWN_SUFFIX,
        }
    }

    /// Returns all compiler-recognized source-file kinds.
    pub fn recognized_kinds() -> &'static [SupportedSourceFileKind] {
        const RECOGNIZED_KINDS: &[SupportedSourceFileKind] = &[
            SupportedSourceFileKind {
                extension: LANGUAGE_SOURCE_EXTENSION,
                kind: SourceFileKind::Moth,
            },
            SupportedSourceFileKind {
                extension: CONTENT_EXTENSION,
                kind: SourceFileKind::MothTemplate,
            },
            SupportedSourceFileKind {
                extension: MARKDOWN_EXTENSION,
                kind: SourceFileKind::PlainMarkdown,
            },
        ];

        RECOGNIZED_KINDS
    }
}

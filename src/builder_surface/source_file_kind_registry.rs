//! Builder-declared source file kind registry.
//!
//! WHAT: tracks which non-standard source file kinds the active builder supports.
//! WHY: the compiler owns the canonical language source extension as the built-in source kind;
//!      builders opt into additional file kinds such as template content and plain Markdown
//!      through `BuilderSurface` so support is builder-controlled.

use crate::projects::settings::{
    CONTENT_EXTENSION, CONTENT_SUFFIX, LANGUAGE_SOURCE_EXTENSION, LANGUAGE_SOURCE_SUFFIX,
    MARKDOWN_EXTENSION, MARKDOWN_SUFFIX,
};
use std::collections::HashMap;

/// Identifies a category of source file that the compiler can ingest.
///
/// WHAT: distinguishes built-in language source from builder-supported extensions.
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

/// A single registered source file kind mapping.
///
/// WHAT: pairs a file extension with its source kind for lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupportedSourceFileKind {
    pub extension: &'static str,
    pub kind: SourceFileKind,
}

/// Registry of builder-supported source file kinds.
///
/// WHAT: collects extensions the active builder wants the compiler to recognize.
/// WHY: keeps source-kind support declarative and builder-local instead of hard-coding
///      extensions in Stage 0 or dependency resolution.
///
/// `.moth` is always implicitly supported and does not need registration.
#[derive(Clone, Debug, Default)]
pub struct SourceFileKindRegistry {
    kinds: HashMap<&'static str, SourceFileKind>,
}

impl SourceFileKindRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            kinds: HashMap::new(),
        }
    }

    /// Registers a source file kind for the given extension.
    ///
    /// WHY: builders declare support so the compiler can discover and handle non-standard
    ///      source files during module building.
    pub fn register(&mut self, extension: &'static str, kind: SourceFileKind) {
        self.kinds.insert(extension, kind);
    }

    /// Looks up the source kind for a file extension.
    ///
    /// Returns `None` for unrecognized extensions. Callers should treat the canonical
    /// language source extension as `SourceFileKind::Moth` even when this returns `None`.
    pub fn kind_for_extension(&self, extension: &str) -> Option<SourceFileKind> {
        self.kinds.get(extension).copied()
    }

    /// Returns whether the given extension is registered as a supported source kind.
    pub fn is_supported(&self, extension: &str) -> bool {
        self.kinds.contains_key(extension)
    }

    /// Returns all registered supported source kinds.
    pub fn supported_kinds(&self) -> Vec<SupportedSourceFileKind> {
        let mut supported_kinds: Vec<_> = self
            .kinds
            .iter()
            .map(|(&extension, &kind)| SupportedSourceFileKind { extension, kind })
            .collect();

        supported_kinds.sort_by_key(|kind| kind.extension);
        supported_kinds
    }

    /// Returns whether this registry supports a recognized source-file extension.
    ///
    /// The language source extension is compiler-owned and always supported.
    /// Builder-owned source kinds must be explicitly registered by the active builder.
    pub fn supports_recognized_extension(&self, extension: &str) -> bool {
        match SourceFileKind::from_extension(extension) {
            Some(SourceFileKind::Moth) => true,
            Some(kind) => self.kind_for_extension(extension) == Some(kind),
            None => false,
        }
    }
}

impl SourceFileKind {
    /// Looks up compiler-recognized source-file kinds by extension.
    ///
    /// WHAT: separates recognition from active-builder support.
    /// WHY: Stage 0 must diagnose a known but unsupported source kind, such as template content
    ///      under a non-HTML builder, instead of falling through to a missing-dependency error.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            LANGUAGE_SOURCE_EXTENSION => Some(Self::Moth),
            CONTENT_EXTENSION => Some(Self::MothTemplate),
            MARKDOWN_EXTENSION => Some(Self::PlainMarkdown),
            _ => None,
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

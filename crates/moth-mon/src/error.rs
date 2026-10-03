//! Structured failures and caller-input byte spans for the MON service.

use std::fmt;

/// A half-open byte range in the caller-provided UTF-8 input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub(super) const fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }
}

/// A location component in an owned MON diagnostic path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Field(String),
    Index(usize),
    MapKey(String),
    Variant(String),
}

/// Stable failure lanes exposed by the MON service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MonErrorCode {
    InvalidUtf8,
    UnexpectedEnd,
    UnexpectedToken,
    MissingComma,
    RootNotRecord,
    TrailingInput,
    InvalidIdentifier,
    InvalidEscape,
    InvalidCharacter,
    NumericSyntax,
    NumericType,
    NumericRange,
    NumericScale,
    NonFiniteFloat,
    UnknownField,
    MissingField,
    DuplicateField,
    TypeMismatch,
    MapKind,
    InvalidMapKey,
    DuplicateMapKey,
    UnknownVariant,
    QualifierMismatch,
    UnknownArgument,
    DuplicateArgument,
    ArgumentOrder,
    Arity,
    UnsupportedSchema,
    InvalidSchema,
    InvalidDefault,
    InputBudget,
    DepthBudget,
    NodeBudget,
    NumericBudget,
    DecodedBudget,
    OutputBudget,
    DefaultBudget,
    InternalInvariant,
}

/// A structured first failure from schema preparation, MON encoding, or complete-document decoding.
///
/// The error owns its code, path and detail independently of the caller's input.
/// Its [`std::fmt::Display`] implementation renders those facts and any byte span,
/// without retaining or reconstructing source excerpts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonError {
    pub code: MonErrorCode,
    pub span: Option<Span>,
    pub path: Vec<PathSegment>,
    pub detail: String,
}

impl MonError {
    pub(super) fn new(
        code: MonErrorCode,
        span: Option<Span>,
        path: &[PathSegment],
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            span,
            path: path.to_owned(),
            detail: detail.into(),
        }
    }

    pub(super) fn at(code: MonErrorCode, span: Span, detail: impl Into<String>) -> Self {
        Self::new(code, Some(span), &[], detail)
    }
}

impl fmt::Display for MonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?} at $", self.code)?;
        for segment in &self.path {
            match segment {
                PathSegment::Field(name) => write!(formatter, ".{name}")?,
                PathSegment::Index(index) => write!(formatter, "[{index}]")?,
                PathSegment::MapKey(key) => write!(formatter, "[{key:?}]")?,
                PathSegment::Variant(name) => write!(formatter, "::{name}")?,
            }
        }
        if let Some(span) = self.span {
            write!(formatter, " (bytes {}..{})", span.start, span.end)?;
        }
        write!(formatter, ": {}", self.detail)
    }
}

impl std::error::Error for MonError {}

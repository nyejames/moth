//! Rust-only MON data ownership and decoder boundary.
//!
//! This module stays beside module compilation. It owns caller-borrowed input cursors,
//! compiler-independent byte spans, immutable host schemas and owned values; it never
//! constructs compiler source identities, AST/HIR values or evaluates expressions.

mod reader;
mod schema;
mod writer;

#[allow(unused_imports)]
pub(crate) use reader::decode_document;
#[allow(unused_imports)]
pub(crate) use schema::prepare_schema;
#[allow(unused_imports)]
pub(crate) use writer::{
    WriteOptions, encode_document, encode_document_with_options, encode_value,
    encode_value_with_options,
};
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

/// A structured first failure from schema preparation or complete-document decoding.
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

    pub(super) fn with_path(
        code: MonErrorCode,
        span: Option<Span>,
        path: &[PathSegment],
        detail: impl Into<String>,
    ) -> Self {
        Self::new(code, span, path, detail)
    }
}

/// Finite resource policy applied before parsing and while expanding owned data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_numeric_digits: usize,
    pub max_decoded_bytes: usize,
    pub max_output_bytes: usize,
    pub max_default_expansions: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_depth: 64,
            max_nodes: 100_000,
            max_numeric_digits: 4_096,
            max_decoded_bytes: 16 * 1024 * 1024,
            max_output_bytes: 16 * 1024 * 1024,
            max_default_expansions: 10_000,
        }
    }
}

/// Owned MON data. The tree intentionally carries no source identity or alias relationship.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Char(char),
    String(String),
    Int(i32),
    Float(f64),
    Integer(String),
    Decimal(String),
    Record(Vec<(String, Value)>),
    Collection(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Choice {
        qualifier: Option<String>,
        variant: String,
        fields: Vec<(String, Value)>,
    },
}

/// One closed-schema record field and its optional exact default value.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub(super) name: String,
    pub(super) ty: SchemaType,
    pub(super) default: Option<Value>,
}

impl Field {
    pub fn required(name: impl Into<String>, ty: SchemaType) -> Self {
        Self {
            name: name.into(),
            ty,
            default: None,
        }
    }

    pub fn with_default(name: impl Into<String>, ty: SchemaType, default: Value) -> Self {
        Self {
            name: name.into(),
            ty,
            default: Some(default),
        }
    }
}

/// One nominal choice variant. Payload fields never have defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub(super) name: String,
    pub(super) fields: Vec<Field>,
}

impl Variant {
    pub fn unit(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
        }
    }

    pub fn payload(name: impl Into<String>, fields: Vec<Field>) -> Self {
        Self {
            name: name.into(),
            fields,
        }
    }
}

/// Static data shape supplied by the Rust consumer.
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaType {
    None,
    Bool,
    Char,
    String,
    Int,
    Float,
    Integer,
    Decimal {
        scale: u8,
    },
    Optional(Box<SchemaType>),
    Record {
        fields: Vec<Field>,
    },
    Struct {
        name: String,
        fields: Vec<Field>,
    },
    Collection {
        element: Box<SchemaType>,
    },
    Map {
        key: Box<SchemaType>,
        value: Box<SchemaType>,
    },
    Choice {
        name: String,
        variants: Vec<Variant>,
    },
    Unsupported {
        name: String,
    },
}

/// Unprepared static schema plus receiver limits.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub(super) root: SchemaType,
    pub(super) limits: Limits,
}

impl Schema {
    pub fn record(fields: Vec<Field>) -> Self {
        Self {
            root: SchemaType::Record { fields },
            limits: Limits::default(),
        }
    }

    pub fn value(ty: SchemaType) -> Self {
        Self {
            root: ty,
            limits: Limits::default(),
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn prepare(self) -> Result<PreparedSchema, MonError> {
        prepare_schema(self)
    }
}

/// Immutable, validated schema reusable across decode calls.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedSchema {
    root: schema::PreparedType,
    limits: Limits,
}

impl PreparedSchema {
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}
#[derive(Debug)]
pub(super) struct BudgetState<'a> {
    pub(super) limits: &'a Limits,
    pub(super) nodes: usize,
    pub(super) decoded_bytes: usize,
    pub(super) default_expansions: usize,
}

impl<'a> BudgetState<'a> {
    pub(super) fn new(limits: &'a Limits) -> Self {
        Self {
            limits,
            nodes: 0,
            decoded_bytes: 0,
            default_expansions: 0,
        }
    }

    pub(super) fn charge_nodes(
        &mut self,
        count: usize,
        span: Option<Span>,
    ) -> Result<(), MonError> {
        let next = self.nodes.checked_add(count).ok_or_else(|| {
            MonError::new(
                MonErrorCode::NodeBudget,
                span,
                &[],
                "MON node budget arithmetic overflowed",
            )
        })?;
        if next > self.limits.max_nodes {
            return Err(MonError::new(
                MonErrorCode::NodeBudget,
                span,
                &[],
                format!("MON node budget exceeded (limit {})", self.limits.max_nodes),
            ));
        }
        self.nodes = next;
        Ok(())
    }

    pub(super) fn charge_decoded_bytes(
        &mut self,
        count: usize,
        span: Option<Span>,
    ) -> Result<(), MonError> {
        let next = self.decoded_bytes.checked_add(count).ok_or_else(|| {
            MonError::new(
                MonErrorCode::DecodedBudget,
                span,
                &[],
                "MON decoded-byte budget arithmetic overflowed",
            )
        })?;
        if next > self.limits.max_decoded_bytes {
            return Err(MonError::new(
                MonErrorCode::DecodedBudget,
                span,
                &[],
                format!(
                    "MON decoded-byte budget exceeded (limit {})",
                    self.limits.max_decoded_bytes
                ),
            ));
        }
        self.decoded_bytes = next;
        Ok(())
    }

    pub(super) fn charge_default(&mut self, span: Option<Span>) -> Result<(), MonError> {
        let next = self.default_expansions.checked_add(1).ok_or_else(|| {
            MonError::new(
                MonErrorCode::DefaultBudget,
                span,
                &[],
                "MON default-expansion budget arithmetic overflowed",
            )
        })?;
        if next > self.limits.max_default_expansions {
            return Err(MonError::new(
                MonErrorCode::DefaultBudget,
                span,
                &[],
                format!(
                    "MON default-expansion budget exceeded (limit {})",
                    self.limits.max_default_expansions
                ),
            ));
        }
        self.default_expansions = next;
        Ok(())
    }

    pub(super) fn check_depth(&self, depth: usize, span: Option<Span>) -> Result<(), MonError> {
        if depth > self.limits.max_depth {
            return Err(MonError::new(
                MonErrorCode::DepthBudget,
                span,
                &[],
                format!(
                    "MON nesting depth exceeded (limit {})",
                    self.limits.max_depth
                ),
            ));
        }
        Ok(())
    }
}

//! Public owned MON values and schema descriptions.
//!
//! This leaf contains consumer-facing data only. Preparation and codec operations live in the
//! schema, reader, and writer owners.

use moth_lexical::numeric::fixed_scalar::FixedScalar;

use super::{Limits, MonError};

/// Owned MON data. The tree intentionally carries no source identity or alias relationship.
///
/// The numeric payload carriers are fixed by this public type: each fixed-width variant uses its
/// own Rust width (`I64` holds `i64`, `U64` holds `u64`), and each binary-float variant uses the
/// `f64` carrier holding a value already rounded to its own precision.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Char(char),
    String(String),
    Integer(String),
    Decimal(String),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    /// A binary16 value, held in the `f64` carrier as an exact binary16 value.
    F16(f64),
    /// A binary32 value, held in the `f64` carrier as an exact binary32 value.
    F32(f64),
    F64(f64),
    /// The `Byte` octet value, which is whole-literal data outside numeric arithmetic.
    Byte(u8),
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
///
/// Every scalar is an explicit fixed identity (`I8`..`I64`, `U8`..`U64`, `F16`..`F64` and `Byte`),
/// so a schema's meaning never depends on configuration. `Integer` and `Decimal` are exact data.
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaType {
    None,
    Bool,
    Char,
    String,
    Integer,
    /// Exact decimal data with a declared scale capacity of `0..=256`.
    Decimal {
        scale: u16,
    },
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F16,
    F32,
    F64,
    /// The `Byte` octet shape: whole-literal `0..=255` data.
    Byte,
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
impl Value {
    /// The explicit fixed scalar this value carries, if it carries a fixed-scalar payload.
    pub(super) fn fixed_scalar(&self) -> Option<FixedScalar> {
        match self {
            Value::I8(_) => Some(FixedScalar::I8),
            Value::I16(_) => Some(FixedScalar::I16),
            Value::I32(_) => Some(FixedScalar::I32),
            Value::I64(_) => Some(FixedScalar::I64),
            Value::U8(_) => Some(FixedScalar::U8),
            Value::U16(_) => Some(FixedScalar::U16),
            Value::U32(_) => Some(FixedScalar::U32),
            Value::U64(_) => Some(FixedScalar::U64),
            Value::F16(_) => Some(FixedScalar::F16),
            Value::F32(_) => Some(FixedScalar::F32),
            Value::F64(_) => Some(FixedScalar::F64),
            Value::Byte(_) => Some(FixedScalar::Byte),
            _ => None,
        }
    }
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
        super::schema::prepare_schema(self)
    }
}

/// Immutable, validated schema reusable across decode calls.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedSchema {
    pub(super) root: super::schema::PreparedType,
    pub(super) limits: Limits,
}

impl PreparedSchema {
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}

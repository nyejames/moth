//! Public owned MON values and schema descriptions.
//!
//! This leaf contains consumer-facing data only. Preparation and codec operations live in the
//! schema, reader, and writer owners.

use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::NumericProfile;

use super::{Limits, MonError};

/// Owned MON data. The tree intentionally carries no source identity or alias relationship.
///
/// The numeric payload carriers are fixed by this public type:
/// - `Int` is `i64` because a schema prepared for an `Int64` profile must hold its whole range;
///   the receiving schema's captured profile decides which values are valid.
/// - `Uint` is `u64` because a schema prepared for a 64-bit `Int` width must hold values above
///   `i64::MAX`; the captured profile decides which values this receiving boundary accepts.
/// - `Float` stays `f64` because that carrier holds every `Float32` value exactly; the captured
///   profile decides rounding and validity, exactly as it does for a decoded literal.
/// - Explicit-width values use their own Rust width, and fixed binary floats use the `f64` carrier
///   holding a value already rounded to their own precision. Explicit-width schemas are
///   profile-independent.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Char(char),
    String(String),
    Int(i64),
    Uint(u64),
    Float(f64),
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
/// `Int`, `Uint` and `Float` are profile-dependent: their prepared nodes capture the receiving
/// `NumericProfile`, which defaults to `NumericProfile::STANDARD`. `Uint` follows the profile's
/// `Int` width, so the default is Uint32. `I8`..`F64` and `Byte` are
/// explicit identities and never alias `Int`, `Uint` or `Float`, even under a matching profile.
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaType {
    None,
    Bool,
    Char,
    String,
    Int,
    /// Profile-dependent unsigned integer following the captured `Int` width.
    Uint,
    Float,
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
    ///
    /// `Int`, `Uint` and `Float` return `None`: they are profile-dependent families whose schema
    /// decides validity, so a value of one profile width never satisfies an explicit-width schema.
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
/// Unprepared static schema plus receiver limits and the numeric profile to prepare under.
///
/// Standalone Rust consumers that never select a profile get `NumericProfile::STANDARD`, which is
/// the delivered `Int32`/`Float64` boundary (`Uint` follows the `Int` width there too). A
/// compilation boundary that types its own `Int`, `Uint` and `Float` differently passes its
/// profile with [`Schema::with_profile`]; explicit-width members ignore it.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub(super) root: SchemaType,
    pub(super) limits: Limits,
    pub(super) profile: NumericProfile,
}

impl Schema {
    pub fn record(fields: Vec<Field>) -> Self {
        Self {
            root: SchemaType::Record { fields },
            limits: Limits::default(),
            profile: NumericProfile::STANDARD,
        }
    }

    pub fn value(ty: SchemaType) -> Self {
        Self {
            root: ty,
            limits: Limits::default(),
            profile: NumericProfile::STANDARD,
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Select the numeric profile this schema's `Int`, `Uint` and `Float` members are prepared under.
    pub fn with_profile(mut self, profile: NumericProfile) -> Self {
        self.profile = profile;
        self
    }

    /// The numeric profile this schema will be prepared under.
    pub fn profile(&self) -> NumericProfile {
        self.profile
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
    pub(super) profile: NumericProfile,
}

impl PreparedSchema {
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The numeric profile this schema's `Int`, `Uint` and `Float` members were prepared under.
    ///
    /// The profile is captured once during preparation; it is not re-read per call, and
    /// explicit-width members do not consult it.
    pub fn profile(&self) -> NumericProfile {
        self.profile
    }
}

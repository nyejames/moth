//! Rust-only MON data ownership and decoder boundary.
//!
//! This module stays beside module compilation. It owns caller-borrowed input cursors,
//! compiler-independent byte spans, immutable host schemas and owned values; it never
//! constructs compiler source identities, AST/HIR values or evaluates expressions.

use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarValue};
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::BinaryFloatPrecision;

mod reader;
mod schema;
mod writer;

pub use reader::{decode_document, decode_document_bytes};
#[allow(unused_imports)]
pub(crate) use schema::prepare_schema;
#[allow(unused_imports)]
pub(crate) use writer::{WriteOptions, encode_document_with_options, encode_value_with_options};
pub use writer::{encode_document, encode_value};
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

/// Finite resource policy applied before parsing and while expanding owned data.
///
/// `max_depth` is bounded by an implementation-safe ceiling so recursive
/// parser, schema, validation and rendering walks stay within native stack.
/// Policies above [`Limits::MAX_SAFE_DEPTH`] are rejected during
/// [`Schema::prepare`] with a structured depth-budget failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    /// Maximum accepted nesting depth. Values above
    /// [`Limits::MAX_SAFE_DEPTH`] are rejected during preparation.
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_numeric_digits: usize,
    pub max_decoded_bytes: usize,
    pub max_output_bytes: usize,
    pub max_default_expansions: usize,
}

impl Limits {
    /// Implementation-safe ceiling for `max_depth`.
    ///
    /// Recursive parser, schema preparation, validation and rendering walks
    /// check depth against the accepted policy, so every prepared schema
    /// stays at or below this bound. There is no unlimited switch.
    pub const MAX_SAFE_DEPTH: usize = 64;
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
///
/// The numeric payload carriers are fixed by this public type:
/// - `Int` is `i64` because a schema prepared for an `Int64` profile must hold its whole range;
///   the receiving schema's captured profile decides which values are valid.
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
/// `Int` and `Float` are profile-dependent: their prepared nodes capture the receiving
/// `NumericProfile`, which defaults to `NumericProfile::STANDARD`. `I8`..`F64` and `Byte` are
/// explicit identities and never alias `Int` or `Float`, even under a matching profile.
#[derive(Clone, Debug, PartialEq)]
pub enum SchemaType {
    None,
    Bool,
    Char,
    String,
    Int,
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
    /// `Int` and `Float` return `None`: they are profile-dependent families whose schema decides
    /// validity, so a value of one profile width never satisfies an explicit-width schema.
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
/// the delivered `Int32`/`Float64` boundary. A compilation boundary that types its own `Int`
/// and `Float` differently passes its profile with [`Schema::with_profile`]; explicit-width
/// members ignore it.
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

    /// Select the numeric profile this schema's `Int` and `Float` members are prepared under.
    pub fn with_profile(mut self, profile: NumericProfile) -> Self {
        self.profile = profile;
        self
    }

    /// The numeric profile this schema will be prepared under.
    pub fn profile(&self) -> NumericProfile {
        self.profile
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
    profile: NumericProfile,
}

impl PreparedSchema {
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The numeric profile this schema's `Int` and `Float` members were prepared under.
    ///
    /// The profile is captured once during preparation; it is not re-read per call, and
    /// explicit-width members do not consult it.
    pub fn profile(&self) -> NumericProfile {
        self.profile
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
/// Render a supported map key and its byte length for diagnostic paths.
fn map_key_name(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Int(number) => number.to_string(),
        Value::I8(number) => number.to_string(),
        Value::I16(number) => number.to_string(),
        Value::I32(number) => number.to_string(),
        Value::I64(number) => number.to_string(),
        Value::U8(number) => number.to_string(),
        Value::U16(number) => number.to_string(),
        Value::U32(number) => number.to_string(),
        Value::U64(number) => number.to_string(),
        Value::Byte(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Char(value) => value.to_string(),
        _ => String::new(),
    }
}

fn map_key_name_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Int(number) => signed_key_len(*number),
        Value::I8(number) => signed_key_len(i64::from(*number)),
        Value::I16(number) => signed_key_len(i64::from(*number)),
        Value::I32(number) => signed_key_len(i64::from(*number)),
        Value::I64(number) => signed_key_len(*number),
        Value::U8(number) => unsigned_key_len(u64::from(*number)),
        Value::U16(number) => unsigned_key_len(u64::from(*number)),
        Value::U32(number) => unsigned_key_len(u64::from(*number)),
        Value::U64(number) => unsigned_key_len(*number),
        Value::Byte(number) => unsigned_key_len(u64::from(*number)),
        Value::Bool(value) => {
            if *value {
                4
            } else {
                5
            }
        }
        Value::Char(value) => value.len_utf8(),
        _ => 0,
    }
}

/// Decimal digit count of a rendered signed key, including its sign.
fn signed_key_len(number: i64) -> usize {
    unsigned_key_len(number.unsigned_abs()) + usize::from(number < 0)
}

/// Decimal digit count of an unsigned magnitude, without allocating its text.
fn unsigned_key_len(mut magnitude: u64) -> usize {
    let mut digits = 1;
    while magnitude >= 10 {
        magnitude /= 10;
        digits += 1;
    }
    digits
}

/// Validation-only duplicate index for MON maps.
///
/// The owned [`Value::Map`] output keeps insertion order; these typed sets only
/// answer “have we seen this key” without rescanning prior entries. Each family keeps its own
/// set, so `Int`, `I8`, `I32`, `I64`, `U64` and `Byte` keys stay distinct even when their
/// numeric values are equal. Lookups borrow the key and never clone it: the decoded-byte budget
/// is charged by the caller before any copy.
#[derive(Debug, Default)]
pub(super) struct MapKeyIndex {
    strings: std::collections::HashSet<String>,
    bools: std::collections::HashSet<bool>,
    chars: std::collections::HashSet<char>,
    ints: std::collections::HashSet<i64>,
    i8s: std::collections::HashSet<i8>,
    i16s: std::collections::HashSet<i16>,
    i32s: std::collections::HashSet<i32>,
    i64s: std::collections::HashSet<i64>,
    u8s: std::collections::HashSet<u8>,
    u16s: std::collections::HashSet<u16>,
    u32s: std::collections::HashSet<u32>,
    u64s: std::collections::HashSet<u64>,
    bytes: std::collections::HashSet<u8>,
}

impl MapKeyIndex {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Returns `None` for a value outside the supported map-key families.
    pub(super) fn contains_value(&self, value: &Value) -> Option<bool> {
        match value {
            Value::String(text) => Some(self.strings.contains(text)),
            Value::Bool(flag) => Some(self.bools.contains(flag)),
            Value::Char(character) => Some(self.chars.contains(character)),
            Value::Int(number) => Some(self.ints.contains(number)),
            Value::I8(number) => Some(self.i8s.contains(number)),
            Value::I16(number) => Some(self.i16s.contains(number)),
            Value::I32(number) => Some(self.i32s.contains(number)),
            Value::I64(number) => Some(self.i64s.contains(number)),
            Value::U8(number) => Some(self.u8s.contains(number)),
            Value::U16(number) => Some(self.u16s.contains(number)),
            Value::U32(number) => Some(self.u32s.contains(number)),
            Value::U64(number) => Some(self.u64s.contains(number)),
            Value::Byte(number) => Some(self.bytes.contains(number)),
            _ => None,
        }
    }

    /// Inserts a supported key, cloning strings only after callers charge the copy.
    pub(super) fn insert_value(&mut self, value: &Value) -> bool {
        match value {
            Value::String(text) => self.strings.insert(text.clone()),
            Value::Bool(flag) => self.bools.insert(*flag),
            Value::Char(character) => self.chars.insert(*character),
            Value::Int(number) => self.ints.insert(*number),
            Value::I8(number) => self.i8s.insert(*number),
            Value::I16(number) => self.i16s.insert(*number),
            Value::I32(number) => self.i32s.insert(*number),
            Value::I64(number) => self.i64s.insert(*number),
            Value::U8(number) => self.u8s.insert(*number),
            Value::U16(number) => self.u16s.insert(*number),
            Value::U32(number) => self.u32s.insert(*number),
            Value::U64(number) => self.u64s.insert(*number),
            Value::Byte(number) => self.bytes.insert(*number),
            _ => false,
        }
    }
}

/// The binary-float precision of a fixed binary float; `None` for integers and `Byte`.
///
/// WHY: [`BinaryFloatPrecision`] owns the rounding and the shortened-format contract, so this is
/// only the identity mapping from a fixed float spelling to its precision.
pub(super) fn fixed_float_precision(scalar: FixedScalar) -> Option<BinaryFloatPrecision> {
    match scalar {
        FixedScalar::F16 => Some(BinaryFloatPrecision::Binary16),
        FixedScalar::F32 => Some(BinaryFloatPrecision::Binary32),
        FixedScalar::F64 => Some(BinaryFloatPrecision::Binary64),
        _ => None,
    }
}

/// The owned public value for one materialised fixed scalar.
///
/// WHY: `FixedScalarValue` owns the exact range and rounding decisions, but its payload is a
///      byte-exact carrier that the public tree does not expose, so decoded values are copied into
///      their own Rust width here.
pub(super) fn fixed_scalar_to_value(materialised: FixedScalarValue) -> Value {
    match materialised.scalar() {
        FixedScalar::I8 => Value::I8(signed_payload(materialised) as i8),
        FixedScalar::I16 => Value::I16(signed_payload(materialised) as i16),
        FixedScalar::I32 => Value::I32(signed_payload(materialised) as i32),
        FixedScalar::I64 => Value::I64(signed_payload(materialised)),
        FixedScalar::U8 => Value::U8(unsigned_payload(materialised) as u8),
        FixedScalar::U16 => Value::U16(unsigned_payload(materialised) as u16),
        FixedScalar::U32 => Value::U32(unsigned_payload(materialised) as u32),
        FixedScalar::U64 => Value::U64(unsigned_payload(materialised)),
        FixedScalar::F16 => Value::F16(float_payload(materialised)),
        FixedScalar::F32 => Value::F32(float_payload(materialised)),
        FixedScalar::F64 => Value::F64(float_payload(materialised)),
        FixedScalar::Byte => Value::Byte(unsigned_payload(materialised) as u8),
    }
}

/// The payload of a materialised fixed scalar that already matches its own scalar identity.
///
/// WHAT: copies the payload of `value` for a fixed scalar that the caller has already checked
///       against the receiving schema, rounding a binary-float payload once at the scalar's own
///       precision and rejecting a non-finite result.
/// WHY:  completion, default preparation and map-key checks must agree on what “this value is
///       valid for this fixed scalar” means, and a fixed binary float materialises with its own
///       rounding contract exactly as its literal does.
pub(super) fn materialize_fixed_value(value: &Value, scalar: FixedScalar) -> Option<Value> {
    let Some(precision) = fixed_float_precision(scalar) else {
        // Integer and octet payloads are `Copy` and already exact in their own carrier width.
        return Some(value.clone());
    };
    let payload = match value {
        Value::F16(payload) | Value::F32(payload) | Value::F64(payload) => *payload,
        _ => return None,
    };
    let rounded = precision.round(payload);
    if !rounded.is_finite() {
        return None;
    }
    match scalar {
        FixedScalar::F16 => Some(Value::F16(rounded)),
        FixedScalar::F32 => Some(Value::F32(rounded)),
        FixedScalar::F64 => Some(Value::F64(rounded)),
        _ => None,
    }
}

/// The signed payload of a materialised signed fixed scalar.
fn signed_payload(materialised: FixedScalarValue) -> i64 {
    materialised
        .as_i64()
        .expect("a signed fixed scalar carries a signed payload")
}

/// The unsigned payload of a materialised unsigned or octet fixed scalar.
fn unsigned_payload(materialised: FixedScalarValue) -> u64 {
    materialised
        .as_u64()
        .expect("an unsigned fixed scalar carries an unsigned payload")
}

/// The binary-float payload of a materialised fixed binary float.
fn float_payload(materialised: FixedScalarValue) -> f64 {
    materialised
        .as_f64()
        .expect("a fixed binary float carries a float payload")
}

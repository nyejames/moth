//! The closed V1 host type vocabulary and the semantic signature of one boundary callable.
//!
//! WHAT: `HostType` names one Moth value type, `HostSignature` holds the ordered parameters and
//! success values plus the builtin `Error!` channel, and `HostFunction` adds the name a builder
//! registers.
//! WHY: the compiler and a native runtime share these semantic values. Wasm carriers, frames and
//! the other physical facts stay with the native host ABI and reach a runtime through the
//! compiler's host manifest.

use std::fmt;

/// A Moth value type a native host function may accept or return.
///
/// `Byte` and `U8` are distinct identities with the same range. `Optional` allows one layer over
/// any other variant. This is the semantic vocabulary. The Wasm carrier for each variant is a
/// physical ABI fact that arrives with the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostType {
    /// The Moth `Bool` type.
    Bool,
    /// The Moth `Char` type.
    Char,
    /// The Moth `I8` type.
    I8,
    /// The Moth `I16` type.
    I16,
    /// The Moth `I32` type.
    I32,
    /// The Moth `I64` type.
    I64,
    /// The Moth `U8` type.
    U8,
    /// The Moth `U16` type.
    U16,
    /// The Moth `U32` type.
    U32,
    /// The Moth `U64` type.
    U64,
    /// The Moth `F32` type.
    F32,
    /// The Moth `F64` type.
    F64,
    /// The Moth `Byte` type, which is distinct from `U8`.
    Byte,
    /// The Moth `String` type.
    String,
    /// One optional layer over another host type, spelled `T?` in Moth source.
    Optional(Box<HostType>),
}

impl HostType {
    /// One optional layer over `inner`.
    pub fn optional(inner: HostType) -> Self {
        Self::Optional(Box::new(inner))
    }
}

impl fmt::Display for HostType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => formatter.write_str("Bool"),
            Self::Char => formatter.write_str("Char"),
            Self::I8 => formatter.write_str("I8"),
            Self::I16 => formatter.write_str("I16"),
            Self::I32 => formatter.write_str("I32"),
            Self::I64 => formatter.write_str("I64"),
            Self::U8 => formatter.write_str("U8"),
            Self::U16 => formatter.write_str("U16"),
            Self::U32 => formatter.write_str("U32"),
            Self::U64 => formatter.write_str("U64"),
            Self::F32 => formatter.write_str("F32"),
            Self::F64 => formatter.write_str("F64"),
            Self::Byte => formatter.write_str("Byte"),
            Self::String => formatter.write_str("String"),
            Self::Optional(inner) => write!(formatter, "{inner}?"),
        }
    }
}

/// The semantic signature of one boundary callable.
///
/// Parameters are ordered. Every parameter is a shared read for the value types the V1
/// vocabulary can express today. Success values are ordered results: a host import declares at
/// most one, while a selected export may return several. The builtin `Error!` channel stays
/// separate from the success values. This is the semantic contract. Wasm carriers, result frames
/// and the other physical facts arrive with the compiler's host manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostSignature {
    parameters: Vec<HostType>,
    success_values: Vec<HostType>,
    fallible: bool,
}

impl HostSignature {
    /// Describes a callable with no parameters, no success values and no error channel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends the next parameter.
    pub fn parameter(mut self, parameter: HostType) -> Self {
        self.push_parameter(parameter);
        self
    }

    /// Appends the next success value.
    pub fn returns(mut self, success: HostType) -> Self {
        self.push_success_value(success);
        self
    }

    /// Declares the builtin `Error!` channel.
    pub fn fallible(mut self) -> Self {
        self.set_fallible();
        self
    }

    /// The parameters in declared order. Each is a shared read for the value types the
    /// vocabulary can express today.
    pub fn parameters(&self) -> &[HostType] {
        &self.parameters
    }

    /// The success values in declared order.
    pub fn success_values(&self) -> &[HostType] {
        &self.success_values
    }

    /// Whether the signature declares the builtin `Error!` channel.
    pub fn is_fallible(&self) -> bool {
        self.fallible
    }

    fn push_parameter(&mut self, parameter: HostType) {
        self.parameters.push(parameter);
    }

    fn push_success_value(&mut self, success: HostType) {
        self.success_values.push(success);
    }

    fn set_fallible(&mut self) {
        self.fallible = true;
    }
}

/// A manual descriptor for one native Rust host function.
///
/// Parameters are shared reads today, because the descriptor vocabulary has no resource access
/// mode yet. A host import returns zero or one success value and may declare the builtin `Error!`
/// channel. The name is the symbol the function is exposed under in its package, and the compiler
/// builds its canonical binding identity from that package path and the name when it produces
/// requirements. The signature is semantic, and physical ABI facts arrive with the compiler's
/// host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFunction {
    name: String,
    signature: HostSignature,
}

impl HostFunction {
    /// Describes a host function exposed under `name` in its package.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            signature: HostSignature::new(),
        }
    }

    /// Appends the next parameter.
    pub fn parameter(mut self, parameter: HostType) -> Self {
        self.signature.push_parameter(parameter);
        self
    }

    /// Declares the success value. A second success value is rejected at registration.
    pub fn returns(mut self, success: HostType) -> Self {
        self.signature.push_success_value(success);
        self
    }

    /// Declares the builtin `Error!` channel.
    pub fn fallible(mut self) -> Self {
        self.signature.set_fallible();
        self
    }

    /// The name the function is exposed under in its package.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The semantic signature shared with the compiler's requirements and the runtime's preflight.
    pub fn signature(&self) -> &HostSignature {
        &self.signature
    }
}

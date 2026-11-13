//! Host contract vocabulary shared by the compiler and native runtimes.
//!
//! WHAT: the closed V1 vocabulary a builder uses to describe one Rust host function: exact
//! scalars except `F16`, `String`, one optional layer and the builtin `Error!` channel.
//! WHY: builders describe host functions without naming compiler-internal signature types, and a
//! native runtime reads the same descriptors without depending on the compiler. The compiler owns
//! the mapping onto its external-package registry, so this crate holds the data both sides must
//! agree on and no Wasmtime or compiler-internal type.

/// A Moth value type a native host function may accept or return.
///
/// `Byte` and `U8` are distinct identities with the same range. `Optional` allows one layer
/// over any other variant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostType {
    Bool,
    Char,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Byte,
    String,
    Optional(Box<HostType>),
}

impl HostType {
    /// One optional layer over `inner`.
    pub fn optional(inner: HostType) -> Self {
        Self::Optional(Box::new(inner))
    }
}

/// A manual descriptor for one native Rust host function.
///
/// Parameters are shared reads. A host function returns zero or one success value and may
/// declare the builtin `Error!` channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFunction {
    name: String,
    parameters: Vec<HostType>,
    returns: Vec<HostType>,
    fallible: bool,
}

impl HostFunction {
    /// Describes a host function exposed under `name` in its package.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: Vec::new(),
            returns: Vec::new(),
            fallible: false,
        }
    }

    /// Appends the next parameter.
    pub fn parameter(mut self, parameter: HostType) -> Self {
        self.parameters.push(parameter);
        self
    }

    /// Declares the success value. A second success value is rejected at registration.
    pub fn returns(mut self, success: HostType) -> Self {
        self.returns.push(success);
        self
    }

    /// Declares the builtin `Error!` channel.
    pub fn fallible(mut self) -> Self {
        self.fallible = true;
        self
    }

    /// The name the function is exposed under in its package.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The parameters in declared order, each a shared read.
    pub fn parameters(&self) -> &[HostType] {
        &self.parameters
    }

    /// The declared success values. Registration rejects more than one.
    pub fn success_values(&self) -> &[HostType] {
        &self.returns
    }

    /// Whether the function declares the builtin `Error!` channel.
    pub fn is_fallible(&self) -> bool {
        self.fallible
    }
}

#[cfg(test)]
#[path = "tests/descriptor_tests.rs"]
mod descriptor_tests;

//! The pure-Rust preflight that matches required imports against declared implementations.
//!
//! WHAT: compares each required host import with the Rust implementation the host declares for
//! its canonical identity and reports every mismatch.
//! WHY: a mismatch must reject the instance before any guest entry, and the runtime needs typed
//! reasons rather than a first-failure summary. This is a descriptor comparison, not a second
//! type checker, and it installs nothing: linker installation stays with Lappet's runtime.
//! Physical ABI facts arrive with the compiler's host manifest.

use crate::{
    HostBindingIdentity, HostImportRequirement, HostRequirements, HostSignature, HostType,
};
use std::fmt;

/// One Rust implementation the host declares for one host binding.
///
/// The identity is the canonical package path plus symbol path a builder registered, and the
/// signature is the semantic contract the host declares for it. The host that installs
/// implementations declares these values from the same `HostFunction` descriptors it registers;
/// Lappet's adapter installation will supply them. Physical ABI facts arrive with the compiler's
/// host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostImplementation {
    identity: HostBindingIdentity,
    signature: HostSignature,
}

impl HostImplementation {
    /// Declares the implementation at `identity` with `signature`.
    pub fn new(identity: HostBindingIdentity, signature: HostSignature) -> Self {
        Self {
            identity,
            signature,
        }
    }

    /// The canonical identity the implementation is declared for.
    pub fn identity(&self) -> &HostBindingIdentity {
        &self.identity
    }

    /// The signature the implementation declares.
    pub fn signature(&self) -> &HostSignature {
        &self.signature
    }
}

/// Checks every required host import against the Rust implementations the host declares.
///
/// The check runs before any guest entry. Each required import must have exactly one
/// implementation under its canonical identity, and that implementation's signature must equal
/// the required one. The preflight compares descriptors for equality only: it is not a type
/// checker. An implementation the artefact does not import is unused, not an error. Every
/// mismatch is collected in canonical import order rather than stopping at the first.
///
/// Exports take no input here: the Rust call API is later runtime design, so an export has no
/// Rust-side declaration yet. The requirements value has already validated export selection.
/// Physical ABI facts arrive with the compiler's host manifest.
pub fn preflight(
    requirements: &HostRequirements,
    implementations: &[HostImplementation],
) -> Result<(), PreflightErrors> {
    let mut errors = Vec::new();

    for import in requirements.imports() {
        let mut matching = implementations
            .iter()
            .filter(|implementation| implementation.identity() == import.identity());
        let Some(implementation) = matching.next() else {
            errors.push(PreflightError::MissingImplementation {
                import: import.identity().clone(),
            });
            continue;
        };
        let additional = matching.count();
        if additional > 0 {
            errors.push(PreflightError::DuplicateImplementation {
                import: import.identity().clone(),
                declared: 1 + additional,
            });
            continue;
        }

        compare_signatures(import, implementation.signature(), &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(PreflightErrors { errors })
    }
}

/// Collects every signature difference between one requirement and its single implementation.
fn compare_signatures(
    import: &HostImportRequirement,
    declared: &HostSignature,
    errors: &mut Vec<PreflightError>,
) {
    let required = import.signature();

    // A count difference already explains any per-index difference, so the index comparison runs
    // only when the two signatures have the same shape.
    if required.parameters().len() != declared.parameters().len() {
        errors.push(PreflightError::ParameterCountMismatch {
            import: import.identity().clone(),
            required: required.parameters().len(),
            declared: declared.parameters().len(),
        });
    } else {
        for (index, (required_type, declared_type)) in required
            .parameters()
            .iter()
            .zip(declared.parameters())
            .enumerate()
        {
            if required_type != declared_type {
                errors.push(PreflightError::ParameterTypeMismatch {
                    import: import.identity().clone(),
                    index,
                    required: required_type.clone(),
                    declared: declared_type.clone(),
                });
            }
        }
    }

    if required.success_values().len() != declared.success_values().len() {
        errors.push(PreflightError::SuccessValueCountMismatch {
            import: import.identity().clone(),
            required: required.success_values().len(),
            declared: declared.success_values().len(),
        });
    } else {
        for (index, (required_type, declared_type)) in required
            .success_values()
            .iter()
            .zip(declared.success_values())
            .enumerate()
        {
            if required_type != declared_type {
                errors.push(PreflightError::SuccessValueTypeMismatch {
                    import: import.identity().clone(),
                    index,
                    required: required_type.clone(),
                    declared: declared_type.clone(),
                });
            }
        }
    }

    if required.is_fallible() != declared.is_fallible() {
        errors.push(PreflightError::ErrorChannelMismatch {
            import: import.identity().clone(),
            required: required.is_fallible(),
            declared: declared.is_fallible(),
        });
    }
}

/// One reason a required host import does not match its declared Rust implementation.
///
/// Every variant carries the canonical import identity, and a type mismatch carries the exact
/// semantic types, so the runtime can attribute the failure without reparsing names.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreflightError {
    /// No implementation was declared for the required import.
    MissingImplementation { import: HostBindingIdentity },
    /// The identity was declared more than once, so the required one is ambiguous.
    DuplicateImplementation {
        import: HostBindingIdentity,
        declared: usize,
    },
    /// The implementation declares a different number of parameters.
    ParameterCountMismatch {
        import: HostBindingIdentity,
        required: usize,
        declared: usize,
    },
    /// A parameter at `index` has a different semantic type.
    ParameterTypeMismatch {
        import: HostBindingIdentity,
        index: usize,
        required: HostType,
        declared: HostType,
    },
    /// The implementation declares a different number of success values.
    SuccessValueCountMismatch {
        import: HostBindingIdentity,
        required: usize,
        declared: usize,
    },
    /// A success value at `index` has a different semantic type.
    SuccessValueTypeMismatch {
        import: HostBindingIdentity,
        index: usize,
        required: HostType,
        declared: HostType,
    },
    /// The requirement and implementation disagree about the builtin `Error!` channel.
    ErrorChannelMismatch {
        import: HostBindingIdentity,
        required: bool,
        declared: bool,
    },
}

impl fmt::Display for PreflightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingImplementation { import } => {
                write!(
                    formatter,
                    "The host import '{import}' has no Rust implementation."
                )
            }
            Self::DuplicateImplementation { import, declared } => write!(
                formatter,
                "The host import '{import}' has {declared} Rust implementations, but exactly one \
                 is required."
            ),
            Self::ParameterCountMismatch {
                import,
                required,
                declared,
            } => write!(
                formatter,
                "The host import '{import}' requires {required} parameter(s), but its Rust \
                 implementation declares {declared}."
            ),
            Self::ParameterTypeMismatch {
                import,
                index,
                required,
                declared,
            } => write!(
                formatter,
                "The host import '{import}' parameter at index {index} requires '{required}', but \
                 its Rust implementation declares '{declared}'."
            ),
            Self::SuccessValueCountMismatch {
                import,
                required,
                declared,
            } => write!(
                formatter,
                "The host import '{import}' requires {required} success value(s), but its Rust \
                 implementation declares {declared}."
            ),
            Self::SuccessValueTypeMismatch {
                import,
                index,
                required,
                declared,
            } => write!(
                formatter,
                "The host import '{import}' success value at index {index} requires \
                 '{required}', but its Rust implementation declares '{declared}'."
            ),
            Self::ErrorChannelMismatch {
                import,
                required,
                declared,
            } => {
                let requirement = if *required {
                    "requires the builtin 'Error!' channel"
                } else {
                    "declares no builtin 'Error!' channel"
                };
                let implementation = if *declared {
                    "declares one"
                } else {
                    "declares none"
                };
                write!(
                    formatter,
                    "The host import '{import}' {requirement}, but its Rust implementation \
                     {implementation}."
                )
            }
        }
    }
}

impl std::error::Error for PreflightError {}

/// Every preflight mismatch, in canonical import order.
///
/// The runtime rejects the instance as a whole and can report each mismatch. Physical ABI facts
/// arrive with the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightErrors {
    errors: Vec<PreflightError>,
}

impl PreflightErrors {
    /// The mismatches in canonical import order.
    pub fn errors(&self) -> &[PreflightError] {
        &self.errors
    }
}

impl fmt::Display for PreflightErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                formatter.write_str("\n")?;
            }
            write!(formatter, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PreflightErrors {}

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

    compare_slots(
        import.identity(),
        HostSlotCategory::Parameter,
        required.parameters(),
        declared.parameters(),
        errors,
    );
    compare_slots(
        import.identity(),
        HostSlotCategory::SuccessValue,
        required.success_values(),
        declared.success_values(),
        errors,
    );

    if required.is_fallible() != declared.is_fallible() {
        errors.push(PreflightError::ErrorChannelMismatch {
            import: import.identity().clone(),
            required: required.is_fallible(),
            declared: declared.is_fallible(),
        });
    }
}

/// Compares one ordered slot list, suppressing positional mismatches when its counts differ.
fn compare_slots(
    import: &HostBindingIdentity,
    category: HostSlotCategory,
    required: &[HostType],
    declared: &[HostType],
    errors: &mut Vec<PreflightError>,
) {
    if required.len() != declared.len() {
        errors.push(PreflightError::SlotCountMismatch {
            import: import.clone(),
            category,
            required: required.len(),
            declared: declared.len(),
        });
    } else {
        for (index, (required_type, declared_type)) in required.iter().zip(declared).enumerate() {
            if required_type != declared_type {
                errors.push(PreflightError::SlotTypeMismatch {
                    import: import.clone(),
                    category,
                    index,
                    required: required_type.clone(),
                    declared: declared_type.clone(),
                });
            }
        }
    }
}

/// The ordered signature list to which a preflight mismatch applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSlotCategory {
    /// A callable parameter.
    Parameter,
    /// A success value, distinct from the error channel.
    SuccessValue,
}

impl fmt::Display for HostSlotCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Parameter => "parameter",
            Self::SuccessValue => "success value",
        })
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
    /// The implementation declares a different number of slots in this category.
    SlotCountMismatch {
        import: HostBindingIdentity,
        category: HostSlotCategory,
        required: usize,
        declared: usize,
    },
    /// A slot at `index` has a different semantic type.
    SlotTypeMismatch {
        import: HostBindingIdentity,
        category: HostSlotCategory,
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
            Self::SlotCountMismatch {
                import,
                category,
                required,
                declared,
            } => write!(
                formatter,
                "The host import '{import}' requires {required} {category}(s), but its Rust \
                 implementation declares {declared}."
            ),
            Self::SlotTypeMismatch {
                import,
                category,
                index,
                required,
                declared,
            } => write!(
                formatter,
                "The host import '{import}' {category} at index {index} requires '{required}', but \
                 its Rust implementation declares '{declared}'."
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

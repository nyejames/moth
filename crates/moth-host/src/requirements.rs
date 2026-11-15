//! The immutable import and export requirements the compiler hands to a runtime.
//!
//! WHAT: one required host import, one selected host-callable export and the checked value that
//! holds both lists in canonical order.
//! WHY: the compiler is the only producer. A runtime consumes requirements without re-sorting or
//! re-deduplicating them, and matches imports by canonical identity rather than by declaration
//! order. Physical ABI facts, such as module and item spellings, Wasm signatures and result
//! frames, arrive with the compiler's host manifest.

use crate::{HostBindingIdentity, HostSignature};
use std::fmt;

/// One host import the compiled artefact requires.
///
/// The identity names the binding a Rust implementation must declare, and the signature is the
/// semantic contract that implementation must match. The physical import name and Wasm signature
/// are physical ABI facts that arrive with the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostImportRequirement {
    identity: HostBindingIdentity,
    signature: HostSignature,
}

impl HostImportRequirement {
    /// Requires the binding at `identity` with `signature`.
    pub fn new(identity: HostBindingIdentity, signature: HostSignature) -> Self {
        Self {
            identity,
            signature,
        }
    }

    /// The canonical identity the runtime matches an implementation by.
    pub fn identity(&self) -> &HostBindingIdentity {
        &self.identity
    }

    /// The semantic signature an implementation must match.
    pub fn signature(&self) -> &HostSignature {
        &self.signature
    }
}

/// One host-callable export the builder selected.
///
/// The facade name selects a concrete public function on the package facade. The export name is
/// the builder-chosen Wasm export name, and the signature is the selected function's semantic
/// contract. Several ordered success values are permitted because exports can return more than a
/// host import may. Physical call wrappers and frames arrive with the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostExportRequirement {
    facade_name: String,
    export_name: String,
    signature: HostSignature,
}

impl HostExportRequirement {
    /// Selects the facade function `facade_name` under the Wasm export name `export_name`.
    pub fn new(
        facade_name: impl Into<String>,
        export_name: impl Into<String>,
        signature: HostSignature,
    ) -> Self {
        Self {
            facade_name: facade_name.into(),
            export_name: export_name.into(),
            signature,
        }
    }

    /// The public facade function this export selects.
    pub fn facade_name(&self) -> &str {
        &self.facade_name
    }

    /// The builder-chosen Wasm export name.
    pub fn export_name(&self) -> &str {
        &self.export_name
    }

    /// The semantic signature of the selected facade function.
    pub fn signature(&self) -> &HostSignature {
        &self.signature
    }
}

/// The immutable host requirements for one Wasm artefact.
///
/// Imports are ordered by canonical identity and exports by Wasm export name, so a consumer
/// never re-sorts or re-deduplicates them. Construction rejects a repeated import identity and a
/// repeated export name. The compiler is the only producer, and physical ABI facts arrive with
/// the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRequirements {
    imports: Vec<HostImportRequirement>,
    exports: Vec<HostExportRequirement>,
}

impl HostRequirements {
    /// Validates and orders the requirement lists.
    ///
    /// `imports` are sorted by canonical identity and `exports` by Wasm export name. A repeated
    /// import identity or export name is rejected, so the returned value is canonical whatever
    /// the input order was.
    pub fn new(
        imports: impl IntoIterator<Item = HostImportRequirement>,
        exports: impl IntoIterator<Item = HostExportRequirement>,
    ) -> Result<Self, RequirementsError> {
        let mut imports = imports.into_iter().collect::<Vec<_>>();
        imports.sort_by(|left, right| left.identity.cmp(&right.identity));
        for pair in imports.windows(2) {
            if pair[0].identity == pair[1].identity {
                return Err(RequirementsError::DuplicateImportIdentity {
                    identity: pair[0].identity.clone(),
                });
            }
        }

        let mut exports = exports.into_iter().collect::<Vec<_>>();
        exports.sort_by(|left, right| {
            left.export_name
                .cmp(&right.export_name)
                .then_with(|| left.facade_name.cmp(&right.facade_name))
        });
        for pair in exports.windows(2) {
            if pair[0].export_name == pair[1].export_name {
                return Err(RequirementsError::DuplicateExportName {
                    export_name: pair[0].export_name.clone(),
                });
            }
        }

        Ok(Self { imports, exports })
    }

    /// The required imports in canonical identity order.
    pub fn imports(&self) -> &[HostImportRequirement] {
        &self.imports
    }

    /// The selected exports in Wasm export-name order.
    pub fn exports(&self) -> &[HostExportRequirement] {
        &self.exports
    }
}

/// Why a requirement list could not be constructed.
///
/// The identity or name is the one the compiler produced twice, so a builder or compiler defect
/// can be attributed without inspecting the input order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequirementsError {
    /// Two import requirements name the same canonical binding identity.
    DuplicateImportIdentity { identity: HostBindingIdentity },
    /// Two export requirements use the same Wasm export name.
    DuplicateExportName { export_name: String },
}

impl fmt::Display for RequirementsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateImportIdentity { identity } => {
                write!(
                    formatter,
                    "The host import '{identity}' is required more than once."
                )
            }
            Self::DuplicateExportName { export_name } => write!(
                formatter,
                "The Wasm export name '{export_name}' is selected more than once."
            ),
        }
    }
}

impl std::error::Error for RequirementsError {}

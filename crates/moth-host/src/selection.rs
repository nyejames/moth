//! The builder's explicit selection of host-callable exports.
//!
//! WHAT: a fluent selection value and the validated, ordered selection it finishes into.
//! WHY: public visibility alone exports nothing, so a builder names each facade function it wants
//! callable from Rust. The selection enforces the name rules the native host ABI fixes without
//! compiler knowledge. Physical export wrappers and frames arrive with the compiler's host
//! manifest.

use std::fmt;

/// One validated export selection: a facade public function and its builder-chosen Wasm name.
///
/// This value only exists after [`HostExports::finish`] has validated the selection. Physical
/// ABI facts arrive with the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostExport {
    facade_name: String,
    export_name: String,
}

impl HostExport {
    /// The public facade function this export selects.
    pub fn facade_name(&self) -> &str {
        &self.facade_name
    }

    /// The builder-chosen Wasm export name.
    pub fn export_name(&self) -> &str {
        &self.export_name
    }
}

/// The builder's explicit selection of host-callable exports, under construction.
///
/// [`export`](Self::export) stays infallible so a builder declares its selection as one fluent
/// value, as the rest of the builder surface does. [`finish`](Self::finish) performs the checks
/// and returns a [`HostExportSelection`], so an invalid selection can't reach the compiler.
/// Physical export wrappers and frames arrive with the compiler's host manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostExports {
    exports: Vec<HostExport>,
}

impl HostExports {
    /// Starts an empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects the public facade function `facade_name` under the Wasm export name `export_name`.
    ///
    /// The same facade function may be selected under more than one export name. The native host
    /// ABI only requires each Wasm export name to be unique within the artefact.
    pub fn export(
        mut self,
        facade_name: impl Into<String>,
        export_name: impl Into<String>,
    ) -> Self {
        self.exports.push(HostExport {
            facade_name: facade_name.into(),
            export_name: export_name.into(),
        });
        self
    }

    /// Validates the selection and returns it ordered by Wasm export name.
    ///
    /// Rejects an empty export name, a name the native host ABI reserves for compiler-emitted
    /// exports, and a repeated export name.
    pub fn finish(mut self) -> Result<HostExportSelection, ExportSelectionError> {
        self.exports.sort_by(|left, right| {
            left.export_name
                .cmp(&right.export_name)
                .then_with(|| left.facade_name.cmp(&right.facade_name))
        });

        // Sorting first makes duplicate detection a neighbour check and gives every rejection a
        // canonical first offender, whatever order the builder wrote the selections in.
        let mut previous: Option<&HostExport> = None;
        for export in &self.exports {
            validate_export_name(export)?;
            if let Some(previous) = previous
                && previous.export_name == export.export_name
            {
                return Err(ExportSelectionError::DuplicateExportName {
                    export_name: export.export_name.clone(),
                });
            }
            previous = Some(export);
        }

        Ok(HostExportSelection {
            exports: self.exports,
        })
    }
}

/// The validated export selection, ordered by Wasm export name.
///
/// Only explicitly selected facade functions may become host-callable exports. Physical ABI
/// facts arrive with the compiler's host manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostExportSelection {
    exports: Vec<HostExport>,
}

impl HostExportSelection {
    /// The selected exports in Wasm export-name order.
    pub fn exports(&self) -> &[HostExport] {
        &self.exports
    }
}

/// Why an export selection could not be finished.
///
/// Every variant names the offending facade function or Wasm export name so the builder can
/// correct the declaration without reading compiler internals.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExportSelectionError {
    /// The selection gave the facade function an empty Wasm export name.
    EmptyExportName { facade_name: String },
    /// The Wasm export name is reserved for compiler-emitted exports.
    ReservedExportName {
        facade_name: String,
        export_name: String,
    },
    /// Two selections use the same Wasm export name.
    DuplicateExportName { export_name: String },
}

impl fmt::Display for ExportSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyExportName { facade_name } => write!(
                formatter,
                "The export selection for facade function '{facade_name}' has an empty Wasm export \
                 name."
            ),
            Self::ReservedExportName {
                facade_name,
                export_name,
            } => write!(
                formatter,
                "The Wasm export name '{export_name}' for facade function '{facade_name}' is \
                 reserved for compiler-emitted exports."
            ),
            Self::DuplicateExportName { export_name } => write!(
                formatter,
                "The Wasm export name '{export_name}' is selected more than once."
            ),
        }
    }
}

impl std::error::Error for ExportSelectionError {}

/// Rejects the names the native host ABI reserves for compiler-emitted exports.
fn validate_export_name(export: &HostExport) -> Result<(), ExportSelectionError> {
    if export.export_name.is_empty() {
        return Err(ExportSelectionError::EmptyExportName {
            facade_name: export.facade_name.clone(),
        });
    }
    if export.export_name.starts_with("moth_")
        || export.export_name.starts_with("__moth")
        || export.export_name == "memory"
    {
        return Err(ExportSelectionError::ReservedExportName {
            facade_name: export.facade_name.clone(),
            export_name: export.export_name.clone(),
        });
    }
    Ok(())
}

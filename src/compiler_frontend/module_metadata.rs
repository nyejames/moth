//! Compiler-frontend-owned module metadata extracted during HIR lowering.
//!
//! WHAT: owns non-HIR compiler metadata that HIR lowering extracts from the AST — resolved
//! documentation fragments — plus the typed lowering result boundary.
//! WHY: these are compiler/builder-facing metadata lanes, not executable semantic HIR state. HIR
//! must carry only executable/semantic IR; documentation fragments belong to the module compilation
//! boundary, not the HIR payload.
//!
//! ## Ownership boundary
//!
//! - `HirLoweringResult` is the typed result boundary returned by HIR lowering. Production
//!   orchestration consumes its named fields and assembles the non-HIR metadata into
//!   `ModuleCompilerMetadata` on the current `Module` payload.
//! - `HirLoweringMetadata` carries only the extracted non-HIR metadata. Successful-module
//!   warnings are not duplicated here: the frontend orchestration `warnings` vector already
//!   merges preparation and AST warnings and remains the single successful-module warning
//!   source. HIR lowering keeps the AST warnings privately in `HirBuilder` for error-context
//!   rendering only.
//! - `ModuleDocFragment` replaces the former `HirDocFragment`. Resolved documentation metadata is
//!   not HIR and uses a non-HIR name and owner.
//! - Documentation-metadata validation lives here, not in HIR validation. Invalid compiler
//!   metadata is an internal `CompilerError` validated before a successful module is returned.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

// -------------------------
//  Resolved documentation fragments
// -------------------------

/// Kind of resolved documentation fragment.
///
/// WHY: preserved so the metadata payload carries the full fragment shape. Currently only `Doc`
/// is produced; the enum remains explicit so future fragment kinds stay a single-owner decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleDocFragmentKind {
    Doc,
}

/// One resolved documentation fragment extracted from the AST.
///
/// WHAT: carries the fully resolved documentation text and its authored source location.
/// WHY: builders and documentation tooling consume resolved doc metadata after HIR lowering. This
///      is compiler metadata, not executable HIR state.
#[derive(Debug, Clone)]
pub struct ModuleDocFragment {
    pub kind: ModuleDocFragmentKind,
    /// The resolved documentation text.
    ///
    /// WHY: preserved for builder/documentation-metadata consumers. Currently read only in tests;
    /// retained so the struct carries the full fragment shape.
    #[allow(dead_code)]
    pub rendered_text: String,
    pub location: SourceLocation,
}

// -------------------------
//  HIR lowering metadata result boundary
// -------------------------

/// Non-HIR compiler metadata extracted by HIR lowering.
///
/// WHAT: bundles resolved documentation fragments that HIR lowering pulls from the AST but must
/// not store on `HirModule`.
/// WHY: HIR owns executable/semantic IR only. This typed metadata lets the build system assemble
/// the module compiler-metadata lane without HIR temporarily carrying non-HIR state.
///      Successful-module warnings are intentionally not duplicated here.
#[derive(Debug, Clone, Default)]
pub struct HirLoweringMetadata {
    pub doc_fragments: Vec<ModuleDocFragment>,
}

impl HirLoweringMetadata {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate resolved documentation-metadata locations.
    ///
    /// WHAT: checks that each doc fragment has a self-consistent source span (start before end).
    /// WHY: invalid compiler metadata is an internal `CompilerError` caught at the module
    ///      compilation boundary before a successful module is returned. This validation moved out
    ///      of HIR validation because documentation fragments are not executable HIR state.
    pub fn validate(&self) -> Result<(), CompilerError> {
        for (index, fragment) in self.doc_fragments.iter().enumerate() {
            // Only resolved `Doc` fragments carry a location that this boundary validates. The
            // kind guard reads `kind` so the fragment-shape field stays live for future kinds.
            if matches!(fragment.kind, ModuleDocFragmentKind::Doc)
                && fragment
                    .location
                    .start_pos
                    .line_number
                    .gt(&fragment.location.end_pos.line_number)
            {
                return Err(doc_fragment_error(
                    &fragment.location,
                    format!(
                        "Doc fragment #{index} has invalid location: start line {} is after end line {}",
                        fragment.location.start_pos.line_number,
                        fragment.location.end_pos.line_number
                    ),
                ));
            }

            if fragment.location.start_pos.line_number == fragment.location.end_pos.line_number
                && fragment.location.start_pos.char_column > fragment.location.end_pos.char_column
            {
                return Err(doc_fragment_error(
                    &fragment.location,
                    format!(
                        "Doc fragment #{index} has invalid location columns: start {} is after end {}",
                        fragment.location.start_pos.char_column,
                        fragment.location.end_pos.char_column
                    ),
                ));
            }
        }

        Ok(())
    }
}

/// Typed HIR lowering result boundary.
///
/// WHAT: bundles the validated `HirModule`, its frontend `TypeEnvironment`, and the extracted
///       non-HIR compiler metadata produced by lowering.
/// WHY: production orchestration consumes these as named fields rather than a positional tuple,
///      keeping the HIR/metadata boundary explicit at the frontend→build-system handoff.
pub struct HirLoweringResult {
    pub hir_module: HirModule,
    pub type_environment: TypeEnvironment,
    pub metadata: HirLoweringMetadata,
}

fn doc_fragment_error(location: &SourceLocation, message: String) -> CompilerError {
    // Invalid non-HIR compiler metadata is an internal compiler invariant failure, not an HIR
    // transformation. Use the general internal compiler error lane.
    CompilerError::new(message, location.clone(), ErrorType::Compiler)
}

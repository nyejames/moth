//! Stage 0 handoff record for one prepared module.
//!
//! WHAT: pairs the compiler-owned [`PreparedModuleInput`] with the scheduling facts Stage 0 needs
//!       between preparation and the module compile call.
//! WHY: preparation produces two different kinds of fact. The semantic payload belongs to the
//!      compiler and travels into module compilation unchanged. Implicit builder-package
//!      activation is a Stage 0 scheduling decision made from the source set that was actually
//!      prepared, so it stays build-owned and never enters the compiler input.

use crate::compiler_frontend::module_compilation::PreparedModuleInput;

/// One prepared module as Stage 0 holds it before scheduling its compile job.
pub(crate) struct PreparedModule {
    /// The provider-independent semantic payload handed to module compilation.
    pub(crate) semantic: PreparedModuleInput,
    /// Whether the selected, actually prepared semantic source set contains a `.mtf` file.
    ///
    /// This is deliberately separate from the compiler input's `source_files`: that table also
    /// retains candidate source identities needed for ownership and diagnostics, while implicit
    /// builder providers may be enabled only by sources Stage 0 actually prepared and reached.
    pub(crate) contains_moth_template: bool,
}

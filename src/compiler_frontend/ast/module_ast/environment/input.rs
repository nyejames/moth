//! Header-stage input contract for AST environment construction.
//!
//! AST environment building consumes the header-built symbol package and dependency visibility as one
//! named value so later phases do not receive loose pieces of header-stage state.

use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;

/// Header-stage outputs consumed by AST environment construction.
///
/// WHAT: bundles module symbols and the header-built binding environment into one named contract.
/// WHY: AST should receive header/dependency-sort output as a single type, not as loose
/// arguments split across `new` and `build`.
pub(crate) struct AstEnvironmentInput {
    pub(crate) module_symbols: ModuleSymbols,
    pub(crate) binding_environment: HeaderBindingEnvironment,
}

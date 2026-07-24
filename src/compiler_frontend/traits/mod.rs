//! Trait syntax, identity, and metadata for the Moth trait subsystem.
//!
//! WHAT: parse-only shells plus resolved compile-time trait metadata.
//! WHY: trait syntax is discovered at the header stage along with other top-level declarations;
//!      AST then resolves stable trait identity and requirement signatures without adding trait
//!      definitions to the ordinary value/type declaration table.

pub(crate) mod definitions;
pub(crate) mod environment;
pub(crate) mod evidence;
pub(crate) mod ids;
pub(crate) mod syntax;

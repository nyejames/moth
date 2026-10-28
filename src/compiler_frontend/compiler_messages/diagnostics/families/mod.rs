//! Family rows, grouped by the compiler stage that owns their diagnostics.
//!
//! WHAT: each child module holds the rows for one group. `registry` joins the groups into one table.
//! WHY: a group keeps its rows beside the diagnostics they describe, so a stage owner edits one file.

pub(crate) mod calls_data;
pub(crate) mod choices_control_flow;
pub(crate) mod generics_traits_api;
pub(crate) mod imports;
pub(crate) mod names_declarations;
pub(crate) mod project_config;
pub(crate) mod templates;
pub(crate) mod token_lexical;
pub(crate) mod types_borrows_targets;

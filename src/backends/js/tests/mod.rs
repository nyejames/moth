//! JavaScript backend semantic correctness tests.
//!
//! These modules pin the observable contract between Moth HIR semantics and emitted JS text.
//! They inspect generated source rather than executing JavaScript, keeping each backend concern in
//! a focused file and sharing direct-HIR construction through `support`.

mod support;

mod assertions;
mod bindings;
mod choices;
mod control_flow;
mod emission_policy;
mod expressions;
mod host;
mod inline_expressions;
mod map_statements;
mod numeric_statements;
mod prelude;
mod reactivity;
mod receiver_methods;
mod results;
mod runtime_helpers;
mod symbols;
mod value_use;

//! JavaScript backend semantic correctness tests.
//!
//! These modules verify emitted backend behavior through direct HIR inputs. Most concerns inspect
//! generated source; runtime numeric invariants execute the emitted helpers in Node.js.

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
mod number_runtime;
mod numeric_carrier;
mod numeric_proofs;
mod numeric_statements;
mod prelude;
mod receiver_methods;
mod results;
mod runtime_helpers;
mod symbols;
mod uint_runtime;
mod value_use;

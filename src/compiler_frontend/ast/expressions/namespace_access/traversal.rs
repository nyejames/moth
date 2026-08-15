//! Namespace-record traversal helpers.
//!
//! WHAT: re-exports the shared namespace-record lookup primitive so the value-position
//!      namespace access parser uses the same member-classification semantics as the
//!      type-position resolver.
//! WHY: the lookup owner lives next to `NamespaceRecord` in the binding environment;
//!      keeping a thin module here avoids scattering the same logic across stages.
//! BOUNDARY: this module does not add new traversal policy; it only exposes the shared
//!      helper to the expression parser.

pub(super) use crate::compiler_frontend::headers::binding_environment::{
    NamespaceMemberLookup, lookup_namespace_member,
};

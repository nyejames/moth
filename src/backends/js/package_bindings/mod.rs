//! JavaScript implementations of builder-provided binding packages.
//!
//! WHAT: maps external package lowering metadata to concrete JS helper emission.
//! WHY: backend-specific package binding code belongs beside the JS backend, while shared package
//! identity and signatures stay in `src/builder_surface`.

pub(crate) mod core;

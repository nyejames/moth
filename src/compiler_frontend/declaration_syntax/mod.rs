//! Reusable declaration shell parsers shared between the header stage and AST.
//!
//! WHAT: `declaration_syntax` owns the syntax parsers that build declaration shells (unresolved
//! structural metadata) for both header top-level declarations and AST body-local declarations.
//! Header parsing stores shells; AST resolves shells.
//!
//! WHY: centralising shell parsing prevents header and AST from rediscovering the same syntax
//! rules independently, and makes the shell/resolution boundary explicit.
//!
//! AST stage and headers also parse function signatures and type annotations the same way,
//! so that logic is centralized in this module.

pub(crate) mod binding_mode;
pub(crate) mod build_config_contract;
pub(crate) mod choice;
pub(crate) mod declaration_shell;
pub(crate) mod generic_parameters;
pub(crate) mod record_body;
pub(crate) mod signature_members;
pub(crate) mod r#struct;
pub(crate) mod type_syntax;

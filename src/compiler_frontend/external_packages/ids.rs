//! Stable identifiers for external symbols across all compiler stages and backends.
//!
//! WHAT: defines the ID types that represent external functions, types, constants, packages,
//! and call targets in HIR and backend lowering. These IDs must remain stable across frontend,
//! analysis, and backend passes.
//! WHY: backends and the borrow checker need to reference external symbols without
//! repeating package-scoped name resolution.

use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::semantic_identity::OriginFunctionId;

pub const CORE_IO_PACKAGE_PATH: &str = "@core/io";
pub const IO_NAMESPACE_NAME: &str = "io";
pub const COLLECTION_GET_HOST_NAME: &str = "__moth_collection_get";
pub const COLLECTION_SET_HOST_NAME: &str = "__moth_collection_set";
pub const COLLECTION_PUSH_HOST_NAME: &str = "__moth_collection_push";
pub const COLLECTION_REMOVE_HOST_NAME: &str = "__moth_collection_remove";
pub const COLLECTION_LENGTH_HOST_NAME: &str = "__moth_collection_length";

/// Stable identifier for an external package within one build.
///
/// WHAT: replaces `&'static str` as the canonical package key so dynamic provider results
/// and built-in packages share the same identity model.
/// WHY: project-local JS imports and future providers need owned, stable identities that
/// are not constrained by compile-time string literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExternalPackageId(pub u32);

/// Stable identifier for an external function across all compiler stages and backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalFunctionId {
    IoPrint,
    IoLine,
    IoDebug,
    IoWarn,
    IoError,
    IoInputNew,
    IoInputUpdate,
    IoInputClose,
    IoInputKeyDown,
    IoInputKeyPressed,
    IoInputKeyReleased,
    IoInputPointerX,
    IoInputPointerY,
    IoInputPointerDown,
    IoInputPointerPressed,
    IoInputPointerReleased,
    IoInputLastKeyPressed,
    IoInputLastKeyReleased,
    IoInputLastPointerPressed,
    IoInputLastPointerReleased,
    CollectionGet,
    CollectionSet,
    CollectionPush,
    CollectionRemove,
    CollectionLength,
    /// Synthetic functions registered by tests. Never emitted by production parsers.
    Synthetic(u32),
}

impl ExternalFunctionId {
    /// Stable JS/helper-facing name for diagnostics and HIR display.
    pub fn name(&self) -> &'static str {
        match self {
            Self::IoPrint => "__moth_io_print",
            Self::IoLine => "__moth_io_line",
            Self::IoDebug => "__moth_io_debug",
            Self::IoWarn => "__moth_io_warn",
            Self::IoError => "__moth_io_error",
            Self::IoInputNew => "__moth_io_input_new",
            Self::IoInputUpdate => "__moth_io_input_update",
            Self::IoInputClose => "__moth_io_input_close",
            Self::IoInputKeyDown => "__moth_io_input_key_down",
            Self::IoInputKeyPressed => "__moth_io_input_key_pressed",
            Self::IoInputKeyReleased => "__moth_io_input_key_released",
            Self::IoInputPointerX => "__moth_io_input_pointer_x",
            Self::IoInputPointerY => "__moth_io_input_pointer_y",
            Self::IoInputPointerDown => "__moth_io_input_pointer_down",
            Self::IoInputPointerPressed => "__moth_io_input_pointer_pressed",
            Self::IoInputPointerReleased => "__moth_io_input_pointer_released",
            Self::IoInputLastKeyPressed => "__moth_io_input_last_key_pressed",
            Self::IoInputLastKeyReleased => "__moth_io_input_last_key_released",
            Self::IoInputLastPointerPressed => "__moth_io_input_last_pointer_pressed",
            Self::IoInputLastPointerReleased => "__moth_io_input_last_pointer_released",
            Self::CollectionGet => COLLECTION_GET_HOST_NAME,
            Self::CollectionSet => COLLECTION_SET_HOST_NAME,
            Self::CollectionPush => COLLECTION_PUSH_HOST_NAME,
            Self::CollectionRemove => COLLECTION_REMOVE_HOST_NAME,
            Self::CollectionLength => COLLECTION_LENGTH_HOST_NAME,
            Self::Synthetic(_) => "<synthetic>",
        }
    }
}

/// Stable identifier for an external type across all compiler stages and backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExternalTypeId(pub u32);

/// Stable identifier for the `io.input.Input` opaque external handle type.
///
/// WHAT: names the single external type used by the core IO input surface so that
///       registry, frontend, HIR, and backend references use one constant value.
/// WHY: prevents raw `ExternalTypeId(...)` values from scattering across the codebase.
pub const IO_INPUT_EXTERNAL_TYPE_ID: ExternalTypeId = ExternalTypeId(1);

/// Stable identifier for an external constant across all compiler stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExternalConstantId(pub u32);

/// Unified identifier for an external symbol visible from a single file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalSymbolId {
    Function(ExternalFunctionId),
    Type(ExternalTypeId),
    Constant(ExternalConstantId),
}

/// Call target for a function invocation in HIR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallTarget {
    Local(FunctionId),
    CrossModule(OriginFunctionId),
    External(ExternalFunctionId),
}

//! Host import identifiers reserved by the Wasm backend.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WasmHostFunction {
    AssertionFailed,
}

impl WasmHostFunction {
    pub(crate) fn module_name(self) -> &'static str {
        match self {
            Self::AssertionFailed => "host",
        }
    }

    pub(crate) fn item_name(self) -> &'static str {
        match self {
            Self::AssertionFailed => "assertion_failed",
        }
    }
}

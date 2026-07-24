//! Caller-supplied Moth template scope placeholders.
//!
//! WHAT: defines the request-side scope shape promised by the direct API without exposing AST
//! folded constants, `StringId`s, `InternedPath`s, or const-record internals.
//! WHY: current compiler-integrated Moth template scope support is built from
//! header/public-surface data. A public conversion for arbitrary folded caller
//! constants needs a separate design so this API remains narrow instead of
//! leaking frontend internals.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MothTemplatePathScope {
    pub(crate) source_path: PathBuf,
    pub(crate) constants: Vec<MothTemplateScopeConstant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MothTemplateScopeConstant {
    _private: (),
}

impl MothTemplateScopeConstant {
    #[cfg(test)]
    pub(crate) fn test_placeholder() -> Self {
        Self { _private: () }
    }
}

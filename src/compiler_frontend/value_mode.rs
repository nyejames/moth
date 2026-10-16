//! Frontend value/access classification.
//!
//! WHAT: carries AST-level mutability/reference/owned-vs-borrowed classification for expressions,
//! declarations, and binding targets.
//!
//! WHY: access and ownership classifications belong to values and bindings, while semantic type
//!      identity is carried separately by `TypeId`.
//!
//! This is not the final runtime ownership flag model. Runtime ownership remains a later lowering
//! concern driven by borrow/last-use analysis.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueMode {
    MutableOwned,
    MutableReference,
    #[default]
    ImmutableOwned,
    ImmutableReference,
}

impl ValueMode {
    pub fn is_mutable(&self) -> bool {
        matches!(self, Self::MutableOwned | Self::MutableReference)
    }

    pub fn as_owned(&self) -> Self {
        match self {
            Self::MutableReference => Self::MutableOwned,
            Self::ImmutableReference => Self::ImmutableOwned,
            _ => *self,
        }
    }
}

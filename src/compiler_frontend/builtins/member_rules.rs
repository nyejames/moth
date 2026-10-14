//! Language-owned collection and map member signatures.
//!
//! The same allocation-free rules operate on AST construction shapes and resolved HIR types.
//! External ABI registrations do not supply these receiver-dependent semantic signatures.

use super::CollectionBuiltinOp;
use super::maps::MapBuiltinOp;

/// Fixed positional argument slots, excluding the receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinArgumentSlots<T: Copy> {
    Zero,
    One(T),
    Two(T, T),
}

impl<T: Copy> BuiltinArgumentSlots<T> {
    pub(crate) fn len(self) -> usize {
        match self {
            Self::Zero => 0,
            Self::One(_) => 1,
            Self::Two(_, _) => 2,
        }
    }

    pub(crate) fn get(self, index: usize) -> Option<T> {
        match (self, index) {
            (Self::One(value) | Self::Two(value, _), 0) => Some(value),
            (Self::Two(_, value), 1) => Some(value),
            _ => None,
        }
    }
}

/// Ordered success slots and recoverable failure stay separate from value type identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuiltinMemberRule<T: Copy> {
    pub(crate) arguments: BuiltinArgumentSlots<T>,
    pub(crate) success: Option<T>,
    pub(crate) fallible: bool,
}

impl CollectionBuiltinOp {
    pub(crate) fn rule<T: Copy>(self, element: T, index: T) -> BuiltinMemberRule<T> {
        use BuiltinArgumentSlots::{One, Two, Zero};
        let (arguments, success, fallible) = match self {
            Self::Get | Self::Remove => (One(index), Some(element), true),
            Self::Set => (Two(index, element), None, true),
            Self::PushGrowable => (One(element), None, false),
            Self::PushFixed => (One(element), None, true),
            Self::Length => (Zero, Some(index), false),
        };
        BuiltinMemberRule {
            arguments,
            success,
            fallible,
        }
    }
}

impl MapBuiltinOp {
    pub(crate) fn rule<T: Copy>(
        self,
        key: T,
        value: T,
        bool_type: T,
        index: T,
    ) -> BuiltinMemberRule<T> {
        use BuiltinArgumentSlots::{One, Two, Zero};
        let (arguments, success, fallible) = match self {
            Self::Get | Self::Remove => (One(key), Some(value), true),
            Self::Contains => (One(key), Some(bool_type), false),
            Self::Set => (Two(key, value), None, true),
            Self::Clear => (Zero, None, false),
            Self::Length => (Zero, Some(index), false),
        };
        BuiltinMemberRule {
            arguments,
            success,
            fallible,
        }
    }
}

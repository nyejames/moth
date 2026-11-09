//! Compact resolved callable return contracts.
//!
//! WHAT: owns the resolved success-slot and error-channel representation shared by every callable
//! surface whose slots are canonical `TypeId`s, and the `Returns | Never` envelope that says
//! whether those slots exist at all.
//! WHY: one contract has exactly one representation. Success slots stay allocation-free for the
//! common zero/one shapes and the error channel remains a separate fallibility fact instead of an
//! ordered element that every consumer must filter by channel. A declared Never contract has no
//! success slots and no error channel, so it must never be confused with an ordinary no-value
//! contract; the enum keeps that distinction unrepresentable as a missing slot.
//!
//! Function type identity and interning depend on the canonical `Multiple` payload (at least two
//! entries), so the success payload is private and only the constructors and accessors below may
//! produce a shape.

use super::ids::TypeId;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::CompilerStage;

/// Ordered success slots of one resolved callable contract.
///
/// `Zero` and `One` stay allocation-free; `Multiple` always holds at least two entries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum SuccessSlots {
    #[default]
    Zero,
    One(TypeId),
    Multiple(Box<[TypeId]>),
}

impl SuccessSlots {
    /// Borrowed view of the ordered success slots.
    ///
    /// WHAT: yields the stored payload without materialising a list.
    /// WHY: callers that accept or return slots consume a slice, so the compact variants need no
    /// scratch allocation.
    pub(crate) fn as_slice(&self) -> &[TypeId] {
        match self {
            SuccessSlots::Zero => &[],
            SuccessSlots::One(type_id) => std::slice::from_ref(type_id),
            SuccessSlots::Multiple(type_ids) => type_ids,
        }
    }
}

/// One resolved ordinary callable contract: ordered success slots and the independent error
/// channel.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CallableReturnShape {
    success: SuccessSlots,
    error: Option<TypeId>,
}

impl CallableReturnShape {
    /// The only raw constructor.
    ///
    /// WHAT: normalises any resolved success sequence to `Zero`, `One` or `Multiple`.
    /// WHY: one contract must have exactly one representation for equality, hashing and function
    /// type interning to stay canonical.
    pub(crate) fn from_slots(
        success: impl IntoIterator<Item = TypeId>,
        error: Option<TypeId>,
    ) -> Self {
        let mut success = success.into_iter();
        let success_slots = match success.next() {
            None => SuccessSlots::Zero,
            Some(first) => match success.next() {
                None => SuccessSlots::One(first),
                Some(second) => {
                    let mut slots = Vec::with_capacity(success.size_hint().0.saturating_add(2));
                    slots.push(first);
                    slots.push(second);
                    slots.extend(success);
                    SuccessSlots::Multiple(slots.into_boxed_slice())
                }
            },
        };
        Self {
            success: success_slots,
            error,
        }
    }

    /// Borrowed view of the ordered success slots.
    pub(crate) fn success(&self) -> &[TypeId] {
        self.success.as_slice()
    }

    /// Canonical type of exactly one success slot, when the contract has exactly one.
    pub(crate) fn single_success_slot(&self) -> Option<TypeId> {
        match &self.success {
            SuccessSlots::One(type_id) => Some(*type_id),
            SuccessSlots::Zero | SuccessSlots::Multiple(_) => None,
        }
    }

    /// Canonical type of the error slot, when the contract is fallible.
    pub(crate) fn error(&self) -> Option<TypeId> {
        self.error
    }

    /// Whether the contract declares an error channel.
    pub(crate) fn is_fallible(&self) -> bool {
        self.error.is_some()
    }

    /// Replace the error channel.
    ///
    /// WHAT: mutates the independent error fact after construction.
    /// WHY: HIR owns a private failure lane that installs or narrows a callable's error channel
    /// once the body is known; the success slots are never rewritten by that lane.
    pub(crate) fn set_error(&mut self, error: Option<TypeId>) {
        self.error = error;
    }

    /// This contract with every canonical type replaced by `f`'s result.
    pub(crate) fn map_type_ids(&self, mut f: impl FnMut(TypeId) -> TypeId) -> Self {
        let success = match &self.success {
            SuccessSlots::Zero => SuccessSlots::Zero,
            SuccessSlots::One(type_id) => SuccessSlots::One(f(*type_id)),
            SuccessSlots::Multiple(type_ids) => {
                SuccessSlots::Multiple(type_ids.iter().map(|type_id| f(*type_id)).collect())
            }
        };
        Self {
            success,
            error: self.error.map(&mut f),
        }
    }
}

/// Whole resolved callable return contract.
///
/// WHAT: states whether a callable completes normally with ordinary success/error slots or never
///       returns to its caller at all.
/// WHY: `Never` is not an empty return shape. Keeping it in the enum stops ordinary consumers from
///      silently treating a non-returning callable as a no-value one, and keeps every ordinary
///      accessor total instead of returning fabricated slots. One envelope serves callable
///      signatures and interned function types, so their identity, substitution and display
///      cannot drift apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FunctionReturnContract {
    Returns(CallableReturnShape),
    /// A declared Never contract. Never carries no type identity of its own, so function type
    /// interning distinguishes it by this variant rather than by a fabricated `TypeId`.
    Never,
}

impl Default for FunctionReturnContract {
    fn default() -> Self {
        Self::Returns(CallableReturnShape::default())
    }
}

impl FunctionReturnContract {
    /// Wrap an ordinary success-slot and error-channel shape.
    pub(crate) fn returns(shape: CallableReturnShape) -> Self {
        Self::Returns(shape)
    }

    /// Build an ordinary contract directly from resolved slots.
    pub(crate) fn from_slots(
        success: impl IntoIterator<Item = TypeId>,
        error: Option<TypeId>,
    ) -> Self {
        Self::returns(CallableReturnShape::from_slots(success, error))
    }

    /// Whether the callable never returns to its caller.
    pub(crate) fn is_never(&self) -> bool {
        matches!(self, Self::Never)
    }

    /// Ordinary error channel, or `None` when the callable declares Never.
    pub(crate) fn error_slot(&self) -> Option<TypeId> {
        self.ordinary().and_then(CallableReturnShape::error)
    }

    /// The ordinary shape, or `None` when the callable declares Never.
    ///
    /// WHAT: never maps Never onto empty slots.
    /// WHY: callers that need a real success/error shape must decide explicitly what a Never
    ///      contract means at their boundary.
    pub(crate) fn ordinary(&self) -> Option<&CallableReturnShape> {
        match self {
            Self::Returns(shape) => Some(shape),
            Self::Never => None,
        }
    }

    /// The ordinary shape for test fixtures that construct an ordinary-only contract.
    ///
    /// WHAT: explicit test-only accessor for fixtures asserting ordinary slots.
    /// WHY: HIR lowering no longer projects Never through an ordinary-only accessor, so no
    ///      production consumer may use this convenience.
    #[cfg(test)]
    pub(crate) fn expect_ordinary(&self) -> &CallableReturnShape {
        self.ordinary().unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "a Never return contract reached an ordinary-only test accessor; use ordinary() and assert the contract variant"
            )
        })
    }

    /// This contract with every canonical type replaced by `f`'s result.
    pub(crate) fn map_type_ids(&self, f: impl FnMut(TypeId) -> TypeId) -> Self {
        match self {
            Self::Returns(shape) => Self::Returns(shape.map_type_ids(f)),
            Self::Never => Self::Never,
        }
    }
}

impl From<CallableReturnShape> for FunctionReturnContract {
    fn from(shape: CallableReturnShape) -> Self {
        Self::Returns(shape)
    }
}

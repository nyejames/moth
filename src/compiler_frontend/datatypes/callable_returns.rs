//! Compact ordinary callable return contracts.
//!
//! WHAT: owns the resolved success-slot and error-channel representation shared by every callable
//! surface whose slots are canonical `TypeId`s.
//! WHY: one contract has exactly one representation. Success slots stay allocation-free for the
//! common zero/one shapes and the error channel remains a separate fallibility fact instead of an
//! ordered element that every consumer must filter by channel.
//!
//! Function type identity and interning depend on the canonical `Multiple` payload (at least two
//! entries), so the success payload is private and only the constructors and accessors below may
//! produce a shape.

use super::ids::TypeId;

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

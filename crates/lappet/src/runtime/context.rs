//! The embedder's context holder and the instance's per-call slot.
//!
//! The holder owns the value between calls. For one guest entry the runtime moves it into the
//! instance and moves it back before returning, including when the call faults or a host import
//! panics. The slot is empty between calls, so one instance can serve the next entry with
//! different context and cannot leak one call's value into the next.
//!
//! A host function names the type it needs. A missing value or another type is a host fault and
//! the function is not called. The slot holds one value, so a mutable borrow cannot conflict
//! with another borrow of the same context.
//!
//! A sendable instance accepts only `Send` context, because its store may move between threads.
//! A thread-bound instance accepts any `'static` value, which is where a game keeps a window or
//! a graphics context.

use super::error::{ContextFault, HostFault};
use super::kind::{InstanceKind, Sendable, ThreadBound};
use std::any::Any;

/// A value that may be stored as context for one instance kind.
pub(crate) trait ContextValue<Kind: InstanceKind>: 'static {
    fn into_context(self) -> Box<Kind::Context>;
}

impl<T: Any + Send> ContextValue<Sendable> for T {
    fn into_context(self) -> Box<dyn Any + Send> {
        Box::new(self)
    }
}

impl<T: Any> ContextValue<ThreadBound> for T {
    fn into_context(self) -> Box<dyn Any> {
        Box::new(self)
    }
}

/// The embedder-owned holder. It allocates when the value is stored, not on each call.
pub(crate) struct HostContext<Kind: InstanceKind = Sendable> {
    value: Option<Box<Kind::Context>>,
}

impl<Kind: InstanceKind> HostContext<Kind> {
    pub(crate) fn new(value: impl ContextValue<Kind>) -> Self {
        Self {
            value: Some(ContextValue::<Kind>::into_context(value)),
        }
    }

    pub(crate) fn empty() -> Self {
        Self { value: None }
    }

    pub(crate) fn get_mut<T: Any>(&mut self) -> Result<&mut T, ContextFault> {
        downcast_slot::<Kind, T>(&mut self.value)
    }

    pub(super) fn take(&mut self) -> Option<Box<Kind::Context>> {
        self.value.take()
    }

    pub(super) fn restore(&mut self, value: Option<Box<Kind::Context>>) {
        self.value = value;
    }
}

/// The per-call slot, borrowed for one host import.
pub(crate) struct CallContext<'a, Kind: InstanceKind = Sendable> {
    slot: &'a mut Option<Box<Kind::Context>>,
}

impl<'a, Kind: InstanceKind> CallContext<'a, Kind> {
    pub(super) fn new(slot: &'a mut Option<Box<Kind::Context>>) -> Self {
        CallContext { slot }
    }

    /// Borrows the context as `T`. Failure means the host function must not run.
    pub(crate) fn require<T: Any>(&mut self) -> Result<&mut T, HostFault> {
        downcast_slot::<Kind, T>(self.slot).map_err(HostFault::Context)
    }
}

/// Borrows the value in a holder or a slot as `T`. Both report the same two failures.
fn downcast_slot<Kind: InstanceKind, T: Any>(
    slot: &mut Option<Box<Kind::Context>>,
) -> Result<&mut T, ContextFault> {
    let value = slot.as_deref_mut().ok_or(ContextFault::Missing)?;
    Kind::downcast_context(value).ok_or(ContextFault::WrongType)
}

//! Instance kinds: the compile-time thread rule for one instance.
//!
//! A sendable instance may move between threads, so its resources, context and host state must
//! be `Send`. A thread-bound instance accepts any `'static` value and the compiler stops it
//! leaving its thread. The kind only selects the trait-object bound stored in the resource table
//! and the context slot. Both kinds share the table, the access rules and the fault handling.

use super::resource::Erased;
use std::any::Any;

/// An instance that may move between threads. Its resources must be `Send`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sendable;

/// An instance that stays on the thread that created it. Its resources may be anything
/// `'static`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ThreadBound;

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Sendable {}
    impl Sealed for super::ThreadBound {}
}

/// The compile-time thread rule for one instance kind.
pub(crate) trait InstanceKind: sealed::Sealed + 'static {
    /// The object bound stored in the one resource table.
    type Object: ?Sized + Erased;
    /// The context bound stored in the per-call slot. A sendable instance accepts only `Send`
    /// context. A thread-bound instance accepts any `'static` value.
    type Context: ?Sized + Any;

    // Each kind names its own trait object, and only the concrete `dyn Any` types carry the
    // downcast, so the two bodies can't be written once.
    fn downcast_context<T: 'static>(value: &mut Self::Context) -> Option<&mut T>;
}

impl InstanceKind for Sendable {
    type Object = dyn Erased + Send;
    type Context = dyn Any + Send;

    fn downcast_context<T: 'static>(value: &mut Self::Context) -> Option<&mut T> {
        value.downcast_mut()
    }
}

impl InstanceKind for ThreadBound {
    type Object = dyn Erased;
    type Context = dyn Any;

    fn downcast_context<T: 'static>(value: &mut Self::Context) -> Option<&mut T> {
        value.downcast_mut()
    }
}

/// The host-state thread rule for one instance kind.
///
/// A sendable instance is `Send` only when its host state is. The bound is on the type, so a
/// value that isn't `Send` cannot be used to create one.
pub(crate) trait HostStateBound<Kind: InstanceKind>: 'static {}

impl<T: Send + 'static> HostStateBound<Sendable> for T {}

impl<T: 'static> HostStateBound<ThreadBound> for T {}

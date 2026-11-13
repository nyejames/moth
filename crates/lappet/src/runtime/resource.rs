//! Per-instance resource table and the process-wide token generator.
//!
//! A token is a nonzero `i64` from one counter shared by every instance in the process. It is
//! never reused and it encodes no address. The table owns each value. Closing removes the entry
//! and returns the value, so the embedder chooses when observable teardown runs. Whatever
//! remains is dropped in ascending token order, which is creation order, because the generator
//! never goes backwards.
use super::error::BoundaryFault;
use super::limits::InstanceLimits;
use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

/// The last token that fits the ABI's `i64` carrier. The counter is a `u64`, but a token the
/// guest cannot be given is not a token.
const LAST_TOKEN: u64 = i64::MAX as u64;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Why an embedder operation on the resource table was refused.
///
/// These are errors, not faults. No guest code ran, so the instance stays usable. Generator
/// exhaustion is the same kind of failure: a host function cannot insert, so exhaustion cannot
/// happen during a guest call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceError {
    /// The process has issued every token that fits the carrier.
    Exhausted,
    /// The instance already holds its live-entry limit.
    Full,
    /// The token is zero, was never issued to this instance, or was closed.
    NotLive,
    /// The token is live, but the value has another type. The entry stays.
    WrongType,
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Exhausted => "the token generator is exhausted",
            Self::Full => "the instance's resource table is full",
            Self::NotLive => "the token is not live in this instance",
            Self::WrongType => "the token's value has another type",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ResourceError {}

/// A process-wide token that remembers its value's type for embedder code.
///
/// The raw `i64` exists at the ABI, where the table check still runs. This wrapper is what the
/// embedder holds, so a `Window` token cannot be passed where a `Texture` is expected.
#[derive(Debug)]
pub(crate) struct ResourceToken<T> {
    raw: NonZeroU64,
    _type: PhantomData<fn() -> T>,
}

impl<T> Copy for ResourceToken<T> {}

impl<T> Clone for ResourceToken<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> PartialEq for ResourceToken<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for ResourceToken<T> {}

impl<T> ResourceToken<T> {
    /// The ABI carrier. Zero is never returned.
    pub(crate) fn raw(self) -> i64 {
        i64::try_from(self.raw.get()).expect("an issued token fits the i64 carrier")
    }

    /// Rebuilds a typed token from bits a guest or another instance could have sent.
    ///
    /// This does not check the table. Zero and a negative carrier are not tokens.
    pub(crate) fn from_raw(raw: i64) -> Option<Self> {
        let raw = u64::try_from(raw).ok()?;
        Some(Self {
            raw: NonZeroU64::new(raw)?,
            _type: PhantomData,
        })
    }
}

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
}

impl InstanceKind for Sendable {
    type Object = dyn Erased + Send;
}

impl InstanceKind for ThreadBound {
    type Object = dyn Erased;
}

/// A stored value the table can check and return without knowing `T` at the map.
pub(crate) trait Erased: Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<T: Any> Erased for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// The host-state thread rule for one instance kind.
///
/// A sendable instance is `Send` only when its host state is. The bound is on the type, so a
/// value that isn't `Send` cannot be used to create one.
pub(crate) trait HostStateBound<Kind: InstanceKind>: 'static {}

impl<T: Send + 'static> HostStateBound<Sendable> for T {}

impl<T: 'static> HostStateBound<ThreadBound> for T {}
/// One live value and its call-scoped borrow, the same rule as `RefCell`.
struct LiveEntry<Object: ?Sized + Erased> {
    value: RefCell<Box<Object>>,
    type_id: std::any::TypeId,
}

/// Live resources for one instance, keyed by token so iteration is creation order.
pub(crate) struct ResourceTable<Object: ?Sized + Erased = dyn Erased + Send> {
    entries: BTreeMap<NonZeroU64, LiveEntry<Object>>,
}

impl<Object: ?Sized + Erased> ResourceTable<Object> {
    pub(crate) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub(crate) fn get<T: 'static>(
        &self,
        token: ResourceToken<T>,
    ) -> Result<Shared<'_, T>, ResourceError> {
        self.borrow_shared(token.raw.get() as i64)
            .map_err(resource_error)
    }

    pub(crate) fn get_mut<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<Exclusive<'_, T>, ResourceError> {
        // The embedder holds `&mut self`, so no host guard can be live. The exclusive borrow
        // is still the same entry state a host call uses.
        self.borrow_exclusive(token.raw.get() as i64)
            .map_err(resource_error)
    }

    /// Resolves a guest token and borrows it shared. The reference exists only after the checks.
    pub(crate) fn borrow_shared<T: 'static>(
        &self,
        raw: i64,
    ) -> Result<Shared<'_, T>, BoundaryFault> {
        let entry = self.live(raw)?;
        self.reject_wrong_type::<T>(entry)?;
        let borrowed = entry
            .value
            .try_borrow()
            .map_err(|_| BoundaryFault::TokenConflict)?;
        let view = Ref::map(borrowed, |value| {
            value
                .as_ref()
                .as_any()
                .downcast_ref::<T>()
                .expect("the type was checked before the borrow")
        });
        Ok(Shared { view })
    }

    /// Resolves a guest token and borrows it exclusively.
    pub(crate) fn borrow_exclusive<T: 'static>(
        &self,
        raw: i64,
    ) -> Result<Exclusive<'_, T>, BoundaryFault> {
        let entry = self.live(raw)?;
        self.reject_wrong_type::<T>(entry)?;
        let borrowed = entry
            .value
            .try_borrow_mut()
            .map_err(|_| BoundaryFault::TokenConflict)?;
        let view = RefMut::map(borrowed, |value| {
            value
                .as_mut()
                .as_any_mut()
                .downcast_mut::<T>()
                .expect("the type was checked before the borrow")
        });
        Ok(Exclusive { view })
    }

    /// Checks every request, then runs `body` with the borrows. A rejected request drops the
    /// borrows already taken and does not call `body`, so no Rust reference exists. The borrows
    /// live in this function, not in the host, so the host cannot keep one by forgetting a guard.
    pub(crate) fn with_access<R>(
        &self,
        requests: &[AccessRequest],
        body: impl FnOnce(&mut PreparedAccess<'_, Object>) -> R,
    ) -> Result<R, BoundaryFault> {
        let mut holds = Vec::with_capacity(requests.len());
        for request in requests {
            holds.push(self.hold(request)?);
        }
        let mut prepared = PreparedAccess { holds };
        Ok(body(&mut prepared))
    }

    fn hold(&self, request: &AccessRequest) -> Result<Held<'_, Object>, BoundaryFault> {
        let entry = self.live(request.raw)?;
        if entry.type_id != request.type_id {
            return Err(BoundaryFault::TokenTypeMismatch);
        }
        if request.exclusive {
            let borrowed = entry
                .value
                .try_borrow_mut()
                .map_err(|_| BoundaryFault::TokenConflict)?;
            Ok(Held::Exclusive(borrowed))
        } else {
            let borrowed = entry
                .value
                .try_borrow()
                .map_err(|_| BoundaryFault::TokenConflict)?;
            Ok(Held::Shared(borrowed))
        }
    }

    /// Removes the entry and returns the value. A wrong type leaves the entry in place.
    pub(crate) fn close<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<T, ResourceError> {
        if self.entry(token.raw)?.type_id != std::any::TypeId::of::<T>() {
            return Err(ResourceError::WrongType);
        }

        let entry = self
            .entries
            .remove(&token.raw)
            .expect("the entry was found before it was removed");
        Ok(*entry
            .value
            .into_inner()
            .into_any()
            .downcast()
            .expect("the type was checked before removal"))
    }

    /// Hands every remaining value back, in creation order, and leaves the table empty.
    pub(crate) fn drain(&mut self) -> DrainedResources {
        let values = std::mem::take(&mut self.entries)
            .into_values()
            .map(|entry| entry.value.into_inner().into_any())
            .collect();
        DrainedResources { values }
    }

    fn insert_boxed<T: 'static>(
        &mut self,
        value: Box<Object>,
        limits: &InstanceLimits,
    ) -> Result<ResourceToken<T>, ResourceError> {
        if self.entries.len() >= limits.max_live_resources {
            return Err(ResourceError::Full);
        }
        let raw = issue_token()?;
        self.entries.insert(
            raw,
            LiveEntry {
                value: RefCell::new(value),
                type_id: std::any::TypeId::of::<T>(),
            },
        );
        Ok(ResourceToken {
            raw,
            _type: PhantomData,
        })
    }

    fn live(&self, raw: i64) -> Result<&LiveEntry<Object>, BoundaryFault> {
        let Some(raw) = u64::try_from(raw).ok().and_then(NonZeroU64::new) else {
            return Err(BoundaryFault::TokenNotLive);
        };
        self.entries.get(&raw).ok_or(BoundaryFault::TokenNotLive)
    }

    fn reject_wrong_type<T: 'static>(
        &self,
        entry: &LiveEntry<Object>,
    ) -> Result<(), BoundaryFault> {
        if entry.type_id == std::any::TypeId::of::<T>() {
            Ok(())
        } else {
            Err(BoundaryFault::TokenTypeMismatch)
        }
    }

    fn entry(&self, raw: NonZeroU64) -> Result<&LiveEntry<Object>, ResourceError> {
        self.entries.get(&raw).ok_or(ResourceError::NotLive)
    }
}

/// A shared borrow of one resource. Dropping it releases the entry.
pub(crate) struct Shared<'a, T> {
    view: Ref<'a, T>,
}

impl<T> Deref for Shared<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.view
    }
}

impl<T: PartialEq> PartialEq for Shared<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        *self.view == *other.view
    }
}

impl<T: fmt::Debug> fmt::Debug for Shared<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.view.fmt(formatter)
    }
}

/// An exclusive borrow of one resource. Dropping it releases the entry.
pub(crate) struct Exclusive<'a, T> {
    view: RefMut<'a, T>,
}

impl<T> Deref for Exclusive<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.view
    }
}

impl<T> DerefMut for Exclusive<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.view
    }
}

/// One token a host call wants to borrow. The type is the parameter's registered type.
pub(crate) struct AccessRequest {
    pub(crate) raw: i64,
    pub(crate) exclusive: bool,
    pub(crate) type_id: std::any::TypeId,
}

enum Held<'a, Object: ?Sized + Erased> {
    Shared(Ref<'a, Box<Object>>),
    Exclusive(RefMut<'a, Box<Object>>),
}

/// Borrows already taken for one host call. A typed reference is created only from here.
pub(crate) struct PreparedAccess<'a, Object: ?Sized + Erased> {
    holds: Vec<Held<'a, Object>>,
}

impl<Object: ?Sized + Erased> PreparedAccess<'_, Object> {
    pub(crate) fn shared<T: 'static>(&self, index: usize) -> &T {
        let Held::Shared(hold) = &self.holds[index] else {
            panic!("access {index} was reserved exclusive");
        };
        hold.as_ref()
            .as_any()
            .downcast_ref()
            .expect("the type was checked before the borrow")
    }

    pub(crate) fn exclusive<T: 'static>(&mut self, index: usize) -> &mut T {
        let Held::Exclusive(hold) = &mut self.holds[index] else {
            panic!("access {index} was reserved shared");
        };
        hold.as_mut()
            .as_any_mut()
            .downcast_mut()
            .expect("the type was checked before the borrow")
    }
}

fn resource_error(fault: BoundaryFault) -> ResourceError {
    match fault {
        BoundaryFault::TokenNotLive => ResourceError::NotLive,
        BoundaryFault::TokenTypeMismatch => ResourceError::WrongType,
        // An embedder lookup holds the instance, so a host guard cannot be live.
        BoundaryFault::TokenConflict => ResourceError::NotLive,
        _ => ResourceError::NotLive,
    }
}

impl ResourceTable<dyn Erased + Send> {
    pub(crate) fn insert<T: Send + 'static>(
        &mut self,
        value: T,
        limits: &InstanceLimits,
    ) -> Result<ResourceToken<T>, ResourceError> {
        self.insert_boxed(Box::new(value), limits)
    }
}

impl ResourceTable<dyn Erased> {
    pub(crate) fn insert<T: 'static>(
        &mut self,
        value: T,
        limits: &InstanceLimits,
    ) -> Result<ResourceToken<T>, ResourceError> {
        self.insert_boxed(Box::new(value), limits)
    }
}

impl<Object: ?Sized + Erased> Drop for ResourceTable<Object> {
    fn drop(&mut self) {
        // `pop_first` is ascending token order. Leaving this to `BTreeMap`'s own drop would
        // also be ordered, but the order is the contract, so it lives in this type.
        while self.entries.pop_first().is_some() {}
    }
}

/// Values recovered from a table, in the order they were inserted.
///
/// The embedder owns them. Dropping this collection drops the values, so recovery takes them
/// out.
pub(crate) struct DrainedResources {
    values: Vec<Box<dyn Any>>,
}

impl IntoIterator for DrainedResources {
    type Item = Box<dyn Any>;
    type IntoIter = std::vec::IntoIter<Box<dyn Any>>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

fn issue_token() -> Result<NonZeroU64, ResourceError> {
    issue_from(&NEXT_TOKEN)
}

fn issue_from(counter: &AtomicU64) -> Result<NonZeroU64, ResourceError> {
    loop {
        let current = counter.load(Ordering::Relaxed);
        if current == 0 || current > LAST_TOKEN {
            return Err(ResourceError::Exhausted);
        }
        let next = if current == LAST_TOKEN {
            0
        } else {
            current + 1
        };
        if counter
            .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            continue;
        }
        // `current` was checked nonzero and at most `i64::MAX`.
        return Ok(NonZeroU64::new(current).expect("a zero token is never issued"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_tokens_are_nonzero_unique_and_then_exhausted() {
        let counter = AtomicU64::new(LAST_TOKEN - 1);
        let first = issue_from(&counter).expect("one token remains before the last");
        let last = issue_from(&counter).expect("the last carrier token can be issued");
        assert_ne!(first, last);
        assert!(first.get() > 0);
        assert_eq!(issue_from(&counter), Err(ResourceError::Exhausted));
        assert_eq!(issue_from(&counter), Err(ResourceError::Exhausted));
    }
}

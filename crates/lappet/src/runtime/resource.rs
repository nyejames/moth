//! Per-instance resource table and the process-wide token generator.
//!
//! A token is a nonzero `i64` from one counter shared by every instance in the process. It is
//! never reused and it encodes no address. The table owns each value. Closing removes the entry
//! and returns the value, so the embedder chooses when observable teardown runs. Whatever
//! remains is dropped in ascending token order, which is creation order, because the generator
//! never goes backwards.
//!
//! The embedder reaches a value through a typed [`ResourceToken`]. A host call reaches it
//! through the raw carrier the guest passed, checked by [`ResourceTable::with_access`].

use super::error::BoundaryFault;
use super::limits::InstanceLimits;
use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};

/// The last token that fits the ABI's `i64` carrier. The counter is a `u64`, but a token the
/// guest cannot be given is not a token.
pub(super) const LAST_TOKEN: u64 = i64::MAX as u64;

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
        formatter.write_str(match self {
            Self::Exhausted => "the token generator is exhausted",
            Self::Full => "the instance's resource table is full",
            Self::NotLive => "the token is not live in this instance",
            Self::WrongType => "the token's value has another type",
        })
    }
}

impl std::error::Error for ResourceError {}

/// A process-wide token that remembers its value's type for embedder code.
///
/// The raw `i64` exists at the ABI, where the table check still runs. This wrapper is what the
/// embedder holds, so a `Window` token cannot be passed where a `Texture` is expected.
pub(crate) struct ResourceToken<T> {
    raw: NonZeroU64,
    _type: PhantomData<fn() -> T>,
}

// Written by hand so a token is `Copy`, comparable and printable whatever `T` is. A derive
// would demand the same trait of `T`, which the token never stores.
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

impl<T> fmt::Debug for ResourceToken<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ResourceToken")
            .field(&self.raw)
            .finish()
    }
}

impl<T> ResourceToken<T> {
    /// The ABI carrier. Zero is never returned.
    pub(crate) fn raw(self) -> i64 {
        i64::try_from(self.raw.get()).expect("an issued token fits the i64 carrier")
    }

    /// Rebuilds a typed token from bits a guest or another instance could have sent.
    ///
    /// This does not check the table. Zero and a negative carrier are not tokens.
    pub(crate) fn from_raw(raw: i64) -> Option<Self> {
        Some(Self {
            raw: carrier_token(raw)?,
            _type: PhantomData,
        })
    }
}

/// The token a raw carrier names. Zero and a negative carrier name none.
fn carrier_token(raw: i64) -> Option<NonZeroU64> {
    u64::try_from(raw).ok().and_then(NonZeroU64::new)
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

/// One live value and its call-scoped borrow, the same rule as `RefCell`.
struct LiveEntry<Object: ?Sized + Erased> {
    value: RefCell<Box<Object>>,
    type_id: TypeId,
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

    // Embedder operations. Each takes a typed token and reports a `ResourceError`.

    /// Resolves a token to its value. The type is checked before the reference exists.
    pub(crate) fn get<T: 'static>(
        &self,
        token: ResourceToken<T>,
    ) -> Result<Shared<'_, T>, ResourceError> {
        let entry = self.entries.get(&token.raw).ok_or(ResourceError::NotLive)?;
        if entry.type_id != TypeId::of::<T>() {
            return Err(ResourceError::WrongType);
        }

        // Only a host call's exclusive hold refuses this borrow. The embedder can't look a
        // token up while a host call runs, so it has no outcome of its own.
        let borrowed = entry
            .value
            .try_borrow()
            .map_err(|_| ResourceError::NotLive)?;
        let view = Ref::map(borrowed, |value| {
            value
                .as_ref()
                .as_any()
                .downcast_ref::<T>()
                .expect("the entry's recorded type was checked before the borrow")
        });
        Ok(Shared { view })
    }

    /// Resolves a token to its value for writing.
    ///
    /// `&mut self` proves no hold is live, so this needs no runtime borrow.
    pub(crate) fn get_mut<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<&mut T, ResourceError> {
        let entry = self
            .entries
            .get_mut(&token.raw)
            .ok_or(ResourceError::NotLive)?;
        if entry.type_id != TypeId::of::<T>() {
            return Err(ResourceError::WrongType);
        }

        Ok(entry
            .value
            .get_mut()
            .as_mut()
            .as_any_mut()
            .downcast_mut::<T>()
            .expect("the entry's recorded type was checked before the borrow"))
    }

    /// Removes the entry and returns the value. A wrong type leaves the entry in place.
    pub(crate) fn close<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<T, ResourceError> {
        let Entry::Occupied(entry) = self.entries.entry(token.raw) else {
            return Err(ResourceError::NotLive);
        };
        if entry.get().type_id != TypeId::of::<T>() {
            return Err(ResourceError::WrongType);
        }

        let value = entry
            .remove()
            .value
            .into_inner()
            .into_any()
            .downcast::<T>()
            .expect("the entry's recorded type was checked before removal");
        Ok(*value)
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
                type_id: TypeId::of::<T>(),
            },
        );
        Ok(ResourceToken {
            raw,
            _type: PhantomData,
        })
    }

    // Guest-carrier operations. Each takes the raw `i64` and reports a `BoundaryFault`.

    /// Checks an export argument before guest entry. A failure leaves the table unchanged.
    pub(crate) fn check_argument(&self, raw: i64, type_id: TypeId) -> Result<(), BoundaryFault> {
        self.live_typed(raw, type_id).map(|_| ())
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
        let entry = self.live_typed(request.raw, request.type_id)?;
        match request.mode {
            AccessMode::Shared => entry
                .value
                .try_borrow()
                .map(Held::Shared)
                .map_err(|_| BoundaryFault::TokenConflict),
            AccessMode::Exclusive => entry
                .value
                .try_borrow_mut()
                .map(Held::Exclusive)
                .map_err(|_| BoundaryFault::TokenConflict),
        }
    }

    /// The live entry a carrier names, once its recorded type matches.
    fn live_typed(&self, raw: i64, type_id: TypeId) -> Result<&LiveEntry<Object>, BoundaryFault> {
        let entry = carrier_token(raw)
            .and_then(|token| self.entries.get(&token))
            .ok_or(BoundaryFault::TokenNotLive)?;
        if entry.type_id != type_id {
            return Err(BoundaryFault::TokenTypeMismatch);
        }
        Ok(entry)
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

/// How a host call borrows one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessMode {
    /// A `&T` parameter. Any number of shared borrows of one entry coexist.
    Shared,
    /// A `&mut T` parameter. It conflicts with every other borrow of the entry.
    Exclusive,
}

/// One token a host call wants to borrow. The type is the parameter's registered type.
pub(crate) struct AccessRequest {
    pub(crate) raw: i64,
    pub(crate) mode: AccessMode,
    pub(crate) type_id: TypeId,
}

enum Held<'a, Object: ?Sized + Erased> {
    Shared(Ref<'a, Box<Object>>),
    Exclusive(RefMut<'a, Box<Object>>),
}

/// Borrows already taken for one host call. A typed reference is created only from here.
///
/// An index is the request's position. Asking for the other mode or another type than the
/// request named is an adapter bug and panics.
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

/// Issues the counter's value and advances it. The exhaustion test passes its own counter.
///
/// Past the last token the counter stops moving, so exhaustion is permanent.
pub(super) fn issue_from(counter: &AtomicU64) -> Result<NonZeroU64, ResourceError> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            (1..=LAST_TOKEN).contains(&next).then(|| next + 1)
        })
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(ResourceError::Exhausted)
}

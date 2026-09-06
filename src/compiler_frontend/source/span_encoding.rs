//! Private bit packing for exact `LocalSpan` words.
//!
//! WHAT: the frozen 22/10 split, niche storage as `logical + 1`, and named encode/decode
//!       functions. No caller outside this module sees a shift or mask.
//! WHY:  raw packing must stay in one place so the census-frozen split cannot drift through
//!       ad hoc bit math in consumers.
//!
//! Slice 1D is the first production caller. Until then this module is reachable only through
//! [`super::span`], which is itself unused outside tests.

#![allow(dead_code)]

use std::num::NonZeroU32;

/// Frozen by the slice 1C2 census. Changing this constant without re-running that census is a
/// contract break: the assertions below pin the measured 22/10 split.
pub(super) const LENGTH_BITS: u32 = 10;

pub(super) const LENGTH_MASK: u32 = (1 << LENGTH_BITS) - 1;
pub(super) const LENGTH_SENTINEL: u32 = LENGTH_MASK;
pub(super) const PAYLOAD_BITS: u32 = 32 - LENGTH_BITS;

/// First start byte that cannot be stored inline.
pub(super) const INLINE_START_LIMIT: u32 = 1 << PAYLOAD_BITS;

/// Largest length that still uses the inline length code.
pub(super) const INLINE_MAX_LENGTH: u32 = LENGTH_SENTINEL - 1;

/// Last zero-based extended-table index that is not the reserved all-ones logical word.
pub(super) const MAX_EXTENDED_INDEX: u32 = (1 << PAYLOAD_BITS) - 2;

const RESERVED_LOGICAL: u32 = u32::MAX;
const MAX_INLINE_LOGICAL: u32 = ((INLINE_START_LIMIT - 1) << LENGTH_BITS) | INLINE_MAX_LENGTH;

const _: () = assert!(LENGTH_BITS == 10);
const _: () = assert!(PAYLOAD_BITS == 22);
const _: () = assert!(LENGTH_MASK == 1023);
const _: () = assert!(LENGTH_SENTINEL == 1023);
const _: () = assert!(INLINE_START_LIMIT == 4 * 1024 * 1024);
const _: () = assert!(INLINE_MAX_LENGTH == 1022);
const _: () = assert!(MAX_EXTENDED_INDEX == 4_194_302);
const _: () = assert!(MAX_INLINE_LOGICAL == RESERVED_LOGICAL - 1);
const _: () = assert!(MAX_INLINE_LOGICAL != RESERVED_LOGICAL);

pub(super) enum DecodedSpan {
    Inline { start: u32, length: u32 },
    Extended { index: u32 },
}

pub(super) fn fits_inline(start: u32, length: u32) -> bool {
    start < INLINE_START_LIMIT && length <= INLINE_MAX_LENGTH
}

pub(super) fn encode_inline(start: u32, length: u32) -> u32 {
    debug_assert!(
        fits_inline(start, length),
        "inline encoding requires a start and length inside the frozen 22/10 limits"
    );

    (start << LENGTH_BITS) | length
}

pub(super) fn encode_extended_index(index: u32) -> Option<u32> {
    if index > MAX_EXTENDED_INDEX {
        return None;
    }

    Some((index << LENGTH_BITS) | LENGTH_SENTINEL)
}

pub(super) fn decode_logical(logical: u32) -> DecodedSpan {
    let length_code = logical & LENGTH_MASK;
    let payload = logical >> LENGTH_BITS;

    if length_code == LENGTH_SENTINEL {
        DecodedSpan::Extended { index: payload }
    } else {
        DecodedSpan::Inline {
            start: payload,
            length: length_code,
        }
    }
}

pub(super) fn store_logical(logical: u32) -> NonZeroU32 {
    let stored = logical.wrapping_add(1);

    NonZeroU32::new(stored)
        .expect("span codec produced the reserved all-ones logical word; this is a compiler bug")
}

pub(super) fn load_logical(stored: NonZeroU32) -> u32 {
    stored.get() - 1
}

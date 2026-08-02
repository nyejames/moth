//! Runtime string helper identifiers.
//!
//! Buffer layout (12 bytes, allocated by `StringNewBuffer`):
//!   offset 0: content_ptr  (i32): pointer to accumulated byte region
//!   offset 4: content_len  (i32): current byte count
//!   offset 8: capacity     (i32): allocated capacity of the content region
//!
//! Finalized string layout (8 bytes, produced by `StringFinish`):
//!   offset 0: ptr (i32): pointer to UTF-8 byte content
//!   offset 4: len (i32): byte length

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WasmRuntimeHelper {
    /// Raw runtime allocator helper returning an address in linear memory.
    Alloc,
    /// Create a 12-byte `{content_ptr, content_len, capacity}` buffer header.
    StringNewBuffer,
    /// Append interned static literal bytes into the buffer (copy-append with grow).
    StringPushLiteral,
    /// Append bytes from a finalized string handle into the buffer (copy-append with grow).
    StringPushHandle,
    /// Materialize an 8-byte finalized `{ptr, len}` string from the buffer.
    StringFinish,
    /// Read the ptr field from a finalized string handle (host ABI bridging).
    StringPtr,
    /// Read the byte length from a finalized string handle.
    StringLen,
    /// Compare two finalized string handles by their UTF-8 byte contents.
    StringEqual,
    /// Convert i64 scalar values into finalized string handles for template interpolation.
    StringFromI64,
    /// Allocate an empty Vec-handle header for `Vec<String>` runtime fragments.
    VecNew,
    /// Append a finalized string handle into a Vec handle.
    VecPushHandle,
    /// Read the logical element count from a Vec handle.
    VecLen,
    /// Read one element handle from a Vec handle.
    VecGet,
    /// Reserved release helper for ownership tuning.
    Release,
    /// Conditional drop hook used at `possible_drop` sites.
    DropIfOwned,
}

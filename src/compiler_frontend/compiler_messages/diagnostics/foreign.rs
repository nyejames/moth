//! Foreign-site references for spans that belong to a different frozen identity than their diagnostic.
//!
//! WHAT: a span from another identity stays in the record's primary field or in a label. A foreign-site row names
//! that slot and the identity context that owns the span. The row holds a selector and a context index, not the span.
//! WHY: frozen identity handles are too large for the hot record. Each distinct resolved identity context is owned
//! by the report and rows refer to it by index. Spans stay in their usual fields, so readers find them in the same
//! place whether or not they are foreign.

use crate::compiler_frontend::source::{FrozenIdentityContext, FrozenIdentityHandle, SourceSpan};

/// Which span of a draft a foreign site belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ForeignSlot {
    Primary,
    Label(usize),
}

/// A foreign site as a constructor supplies it. The span stays in the draft's primary or label field.
#[derive(Clone, Debug)]
pub(super) struct ForeignSiteDraft {
    pub(super) slot: ForeignSlot,
    pub(super) context: FrozenIdentityHandle,
}

/// A foreign site as the store keeps it: a selector and an identity-context index. `selector` is 0 for the primary
/// span and `label + 1` for a label. `context` indexes the report's foreign identity-context list.
#[derive(Clone, Copy, Debug)]
pub(super) struct ForeignSiteRecord {
    pub(super) selector: u32,
    pub(super) context: u32,
}

// Two words: the selector and the context index. The span stays with its record or label, so the row is 8 bytes.
const _: () = assert!(
    size_of::<ForeignSiteRecord>() == 8,
    "foreign sites are locked at 8 bytes"
);

/// A span as a reader sees it: local to the diagnostic, foreign to it, or absent.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SiteRef<'a> {
    Absent,
    Local(SourceSpan),
    Foreign {
        span: SourceSpan,
        /// `None` keeps an unresolved foreign span from falling back to the report's local identity.
        context: Option<&'a FrozenIdentityContext>,
    },
}

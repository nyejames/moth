//! Unit tests for the compact diagnostic store.
//!
//! WHAT: checks codecs, schema rows, family inventory, compaction, capacity limits and stable keys.
//! WHY: the store packs words and checks arena limits by hand, so a layout slip would otherwise
//! surface only when a diagnostic is rendered.

mod boundary_conversion_tests;
mod capacity_tests;
mod codec_tests;
mod compaction_tests;
mod family_inventory_tests;
mod freeze_tests;
mod label_text_tests;
mod remap_tests;
mod report_tests;
mod schema_tests;

use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

/// A span of `length` bytes at `start` in a fixed test source.
pub(super) fn exact_span(start: u32, length: u32) -> SourceSpan {
    let mut extended = ExtendedSpanBuilder::new();
    let local = match LocalSpan::exact(start, length, &mut extended) {
        Ok(local) => local,
        Err(_) => unreachable!("test spans sit far below the local span limit"),
    };
    SourceSpan::new(SourceId::from_index(1), local)
}

/// A fresh foreign identity. Every call returns a handle that compares unequal to earlier ones.
pub(super) fn identity() -> FrozenIdentityHandle {
    FrozenIdentityHandle::new()
}

pub(super) fn frozen_identity_context() -> Arc<FrozenIdentityContext> {
    Arc::new(FrozenIdentityContext::from_parts(
        StringTable::new(),
        SourceDatabase::empty(),
    ))
}

/// A fresh foreign handle already resolved to the supplied frozen context.
pub(super) fn identity_for(context: &Arc<FrozenIdentityContext>) -> FrozenIdentityHandle {
    let handle = identity();
    handle
        .install(Arc::clone(context))
        .expect("test identity accepts its context");
    handle
}

/// A test-only family that declares additional label roles for the label-code tests. It is never registered,
/// so no render path can reach it.
pub(super) static TEST_LABEL_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9001),
    category: DiagnosticCategory::Rule,
    reserved: false,
    descriptor: crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
        "MOTH-TEST-9001",
        "Test family",
        crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error,
    ),
    layout: &[],
    extra: ExtraDataKind::None,
    labels: &[
        LabelMessageCode::PREVIOUS_DECLARATION,
        LabelMessageCode::CONFIG_INPUT_ORIGIN,
        LabelMessageCode::GENERIC_INSTANTIATION_SUBSTITUTIONS,
        LabelMessageCode::TYPED_FAILURE_PRODUCER,
        LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN,
    ],
    render: None,
    reason_key: |_| None,
};

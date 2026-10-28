//! Test-only fixture support for finished expression stores.
//!
//! Parsed fixtures cannot spell synthetic-interface provenance, so HIR regressions reopen a
//! finished store and attach that provenance afterwards. This child module reaches the private
//! owner-token mint, so production code keeps no reopen path.

use super::{AstExpressionBuilder, ExpressionStore, OwnerToken};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;

/// Reopens a frozen store as a builder for fixture mutation.
///
/// Minting a new owner token matches `freeze`, so owned handles taken before the reopen cannot
/// be used with the reopened store.
pub(crate) fn reopen_frozen_store_for_fixture(
    mut storage: ExpressionStore,
) -> Result<AstExpressionBuilder, InfrastructureFailure> {
    storage.owner = OwnerToken::mint()?;
    Ok(AstExpressionBuilder {
        storage,
        candidate_serial: 0,
        limit: u32::MAX,
    })
}

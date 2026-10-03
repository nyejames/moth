//! Resource limits and accounting shared by MON parsing, schema preparation, and rendering.

use super::{MonError, MonErrorCode, Span};

/// Finite resource policy applied before parsing and while expanding owned data.
///
/// `max_depth` is bounded by an implementation-safe ceiling so recursive
/// parser, schema, validation and rendering walks stay within native stack.
/// Policies above [`Limits::MAX_SAFE_DEPTH`] are rejected during
/// [`crate::Schema::prepare`] with a structured depth-budget failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    /// Maximum accepted nesting depth. Values above
    /// [`Limits::MAX_SAFE_DEPTH`] are rejected during preparation.
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_numeric_digits: usize,
    pub max_decoded_bytes: usize,
    pub max_output_bytes: usize,
    pub max_default_expansions: usize,
}

impl Limits {
    /// Implementation-safe ceiling for `max_depth`.
    ///
    /// Recursive parser, schema preparation, validation and rendering walks
    /// check depth against the accepted policy, so every prepared schema
    /// stays at or below this bound. There is no unlimited switch.
    pub const MAX_SAFE_DEPTH: usize = 64;
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_depth: 64,
            max_nodes: 100_000,
            max_numeric_digits: 4_096,
            max_decoded_bytes: 16 * 1024 * 1024,
            max_output_bytes: 16 * 1024 * 1024,
            max_default_expansions: 10_000,
        }
    }
}
#[derive(Debug)]
pub(super) struct BudgetState<'a> {
    pub(super) limits: &'a Limits,
    pub(super) nodes: usize,
    pub(super) decoded_bytes: usize,
    pub(super) default_expansions: usize,
}

impl<'a> BudgetState<'a> {
    pub(super) fn new(limits: &'a Limits) -> Self {
        Self {
            limits,
            nodes: 0,
            decoded_bytes: 0,
            default_expansions: 0,
        }
    }

    pub(super) fn charge_nodes(
        &mut self,
        count: usize,
        span: Option<Span>,
    ) -> Result<(), MonError> {
        let next = self.nodes.checked_add(count).ok_or_else(|| {
            MonError::new(
                MonErrorCode::NodeBudget,
                span,
                &[],
                "MON node budget arithmetic overflowed",
            )
        })?;
        if next > self.limits.max_nodes {
            return Err(MonError::new(
                MonErrorCode::NodeBudget,
                span,
                &[],
                format!("MON node budget exceeded (limit {})", self.limits.max_nodes),
            ));
        }
        self.nodes = next;
        Ok(())
    }

    pub(super) fn charge_decoded_bytes(
        &mut self,
        count: usize,
        span: Option<Span>,
    ) -> Result<(), MonError> {
        let next = self.decoded_bytes.checked_add(count).ok_or_else(|| {
            MonError::new(
                MonErrorCode::DecodedBudget,
                span,
                &[],
                "MON decoded-byte budget arithmetic overflowed",
            )
        })?;
        if next > self.limits.max_decoded_bytes {
            return Err(MonError::new(
                MonErrorCode::DecodedBudget,
                span,
                &[],
                format!(
                    "MON decoded-byte budget exceeded (limit {})",
                    self.limits.max_decoded_bytes
                ),
            ));
        }
        self.decoded_bytes = next;
        Ok(())
    }

    pub(super) fn charge_default(&mut self, span: Option<Span>) -> Result<(), MonError> {
        let next = self.default_expansions.checked_add(1).ok_or_else(|| {
            MonError::new(
                MonErrorCode::DefaultBudget,
                span,
                &[],
                "MON default-expansion budget arithmetic overflowed",
            )
        })?;
        if next > self.limits.max_default_expansions {
            return Err(MonError::new(
                MonErrorCode::DefaultBudget,
                span,
                &[],
                format!(
                    "MON default-expansion budget exceeded (limit {})",
                    self.limits.max_default_expansions
                ),
            ));
        }
        self.default_expansions = next;
        Ok(())
    }

    pub(super) fn check_depth(&self, depth: usize, span: Option<Span>) -> Result<(), MonError> {
        if depth > self.limits.max_depth {
            return Err(MonError::new(
                MonErrorCode::DepthBudget,
                span,
                &[],
                format!(
                    "MON nesting depth exceeded (limit {})",
                    self.limits.max_depth
                ),
            ));
        }
        Ok(())
    }
}

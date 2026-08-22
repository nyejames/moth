//! Terminator-level borrow transfer rules.
//!
//! WHAT: applies borrow effects for branch, match, and return terminators.
//! WHY: terminators join control-flow analysis with expression access checks, so their
//! transfer entrypoint is kept separate from ordinary statement handling.

use super::*;

pub(crate) fn transfer_terminator(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &BorrowState,
    block_id: BlockId,
    terminator: &HirTerminator,
    stats: &mut BlockTransferStats,
    value_fact_buffer: &mut ValueFactBuffer,
) -> Result<(), BorrowCheckError> {
    let mut tracker = StatementAccessTracker::new(layout.local_count());
    let location = context
        .diagnostics
        .terminator_error_location(block_id, terminator);
    let conflicts_before = stats.conflicts_checked;
    let terminator_order = layout.terminator_order_or_unknown(block_id);

    match terminator {
        // Jump argument passing is CFG plumbing, not a semantic read.
        HirTerminator::Jump { .. } => {}

        HirTerminator::If { condition, .. } => {
            let mut read_env = SharedReadEnv {
                context,
                layout,
                state,
                block_id,
                tracker: &mut tracker,
                location: location.clone(),
                current_order: terminator_order,
                stats,
                value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                condition,
                location.clone(),
                &mut RootSet::empty(layout.local_count()),
            )?;
        }

        HirTerminator::FallibleBranch { result, .. } => {
            let mut read_env = SharedReadEnv {
                context,
                layout,
                state,
                block_id,
                tracker: &mut tracker,
                location: location.clone(),
                current_order: terminator_order,
                stats,
                value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                result,
                location.clone(),
                &mut RootSet::empty(layout.local_count()),
            )?;
        }

        HirTerminator::Match { scrutinee, arms } => {
            {
                let mut read_env = SharedReadEnv {
                    context,
                    layout,
                    state,
                    block_id,
                    tracker: &mut tracker,
                    location: location.clone(),
                    current_order: terminator_order,
                    stats,
                    value_fact_buffer,
                };
                record_shared_reads_in_expression(
                    &mut read_env,
                    scrutinee,
                    location.clone(),
                    &mut RootSet::empty(layout.local_count()),
                )?;
            }

            for arm in arms {
                let mut read_env = SharedReadEnv {
                    context,
                    layout,
                    state,
                    block_id,
                    tracker: &mut tracker,
                    location: location.clone(),
                    current_order: terminator_order,
                    stats,
                    value_fact_buffer,
                };
                record_shared_reads_in_pattern(&mut read_env, arm)?;
            }
        }

        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => {
            let mut read_env = SharedReadEnv {
                context,
                layout,
                state,
                block_id,
                tracker: &mut tracker,
                location: location.clone(),
                current_order: terminator_order,
                stats,
                value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                value,
                location.clone(),
                &mut RootSet::empty(layout.local_count()),
            )?;
        }

        HirTerminator::AssertFailure { message, .. } => {
            let mut read_env = SharedReadEnv {
                context,
                layout,
                state,
                block_id,
                tracker: &mut tracker,
                location: location.clone(),
                current_order: terminator_order,
                stats,
                value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                message,
                location.clone(),
                &mut RootSet::empty(layout.local_count()),
            )?;
        }

        HirTerminator::RuntimeFailure { .. } => {
            // Compiler-generated runtime-failure messages are text data, not expressions.
        }

        HirTerminator::Uninitialized => {
            return Err(context.diagnostics.internal_error(
                "Uninitialized terminator reached borrow validation",
                location,
            ));
        }

        HirTerminator::Break { .. } | HirTerminator::Continue { .. } => {}
    }

    stats.terminator_fact = Some((
        block_id,
        TerminatorBorrowFact {
            shared_roots: roots_to_local_ids(layout, &tracker.shared_roots),
            mutable_roots: roots_to_local_ids(layout, &tracker.mutable_roots),
            conflicts_checked: stats.conflicts_checked - conflicts_before,
        },
    ));

    Ok(())
}

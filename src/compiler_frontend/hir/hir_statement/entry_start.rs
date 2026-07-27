//! Entry `start()` runtime fragment lowering.
//!
//! WHAT: initializes the runtime fragment accumulator local for the implicit
//! entry start function and emits its implicit return.
//! WHY: top-level runtime templates are source-order page fragments, but HIR
//! represents them as ordinary runtime string pushes into this accumulator.

use crate::compiler_frontend::ast::ast_nodes::SourceLocation;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::terminators::HirTerminator;

impl<'a> HirBuilder<'a> {
    /// Allocates and initializes the `Vec<String>` fragment accumulator for the
    /// implicit entry `start()` function.
    ///
    /// WHAT: creates an empty collection local before the start body is lowered.
    /// WHY: each `PushStartRuntimeFragment` statement in the body appends one
    ///      evaluated string to this local; the implicit return at end loads it.
    pub(super) fn maybe_initialize_entry_fragment_accumulator(
        &mut self,
        function_id: FunctionId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        if Some(function_id) != self.module.start_function {
            return Ok(());
        }

        let string_ty = builtin_type_ids::STRING;
        let vec_ty = self.type_environment.intern_collection(string_ty, None);
        let vec_local = self.allocate_temp_local(vec_ty, Some(location.clone()))?;

        let region = self.current_region_or_error(location)?;
        let empty_collection = self.make_expression(
            location,
            HirExpressionKind::Collection(vec![]),
            vec_ty,
            ValueKind::RValue,
            region,
        );
        self.emit_assign_local_statement(vec_local, empty_collection, location)?;
        self.entry_fragment_vec_local = Some(vec_local);

        Ok(())
    }

    /// Emits the implicit return of the fragment accumulator for entry `start()`.
    ///
    /// WHAT: loads the fragment vec local and emits `HirTerminator::Return`.
    /// WHY: the entry start body contains only `PushStartRuntimeFragment` nodes
    ///      with no explicit return; the return type is `Vec<String>` consumed
    ///      by the builder as the ordered fragment list.
    ///
    /// Returns `true` only when the return was actually emitted.
    pub(super) fn maybe_emit_entry_fragment_return(
        &mut self,
        function_id: FunctionId,
        current_block: BlockId,
        location: &SourceLocation,
    ) -> Result<bool, CompilerError> {
        if Some(function_id) != self.module.start_function {
            return Ok(false);
        }

        let Some(vec_local) = self.entry_fragment_vec_local else {
            return Ok(false);
        };

        let vec_type = self.local_type_id_or_error(vec_local, location)?;
        let region = self.current_region_or_error(location)?;
        let load_expr = self.make_local_load_expression(vec_local, vec_type, location, region);
        self.emit_terminator(current_block, HirTerminator::Return(load_expr), location)?;

        Ok(true)
    }
}

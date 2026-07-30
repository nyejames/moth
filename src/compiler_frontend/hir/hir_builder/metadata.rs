//! Metadata passes for HIR module construction.
//!
//! WHAT: fills non-CFG module metadata after declarations and constants have
//! been prepared.
//! WHY: function origins are executable HIR metadata consumed by builders and later validation.
//! Resolved documentation fragments are non-HIR compiler metadata extracted into the lowering
//! metadata result boundary, not stored on `HirModule`.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::AstDocFragmentKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::functions::{HirFunctionOrigin, HirStableFunctionOrigin};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::module_metadata::{ModuleDocFragment, ModuleDocFragmentKind};

impl<'a> HirBuilder<'a> {
    /// Accumulate this expression's synthetic-interface provenance into the current function's
    /// direct provenance fact.
    ///
    /// WHAT: merges the expression's `synthetic_interface_provenance` into the current function's
    /// entry in `module.function_provenance`. This reuses the existing expression-lowering
    /// traversal so no separate AST walker is needed.
    /// WHY: the per-function link-fact lane needs the sorted, duplicate-free union of all
    /// expression provenance lowered from the function body. The fact is pre-populated as empty
    /// during declaration registration and accumulates during body lowering.
    pub(crate) fn accumulate_function_provenance(&mut self, expression: &Expression) {
        if expression.synthetic_interface_provenance.is_empty() {
            return;
        }
        let Some(function_id) = self.current_function else {
            return;
        };
        if let Some(provenance) = self.module.function_provenance.get_mut(&function_id) {
            provenance.merge(&expression.synthetic_interface_provenance);
        }
    }

    pub(super) fn assign_function_origins(&mut self) -> Result<(), CompilerError> {
        // WHAT: classify every lowered function and retain stable origins for direct public
        // functions and receiver methods.
        // WHY: downstream public-interface finalization needs an explicit semantic join while
        // backend-facing origin tags remain unchanged for private functions and entry start.
        self.module.function_origins.clear();
        self.module.function_ids_by_origin.clear();
        self.module.function_ids_by_private_origin.clear();

        for function in &self.module.functions {
            self.module
                .function_origins
                .insert(function.id, HirFunctionOrigin::Normal);

            if Some(function.id) == self.module.start_function {
                continue;
            }

            let function_path = self
                .side_table
                .function_name_path(function.id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "HIR function-origin lowering is missing the declaration path for local function {:?}",
                        function.id
                    ))
                })?;

            let Some(origin) = self
                .function_origin_lookup
                .consume_origin_for(function_path)
            else {
                continue;
            };

            match origin {
                HirStableFunctionOrigin::Public(origin) => {
                    if self.module.function_ids_by_origin.contains_key(&origin) {
                        return Err(CompilerError::compiler_error(format!(
                            "HIR function-origin lowering received duplicate stable origin {:?}",
                            origin
                        )));
                    }
                    self.module
                        .function_ids_by_origin
                        .insert(origin, function.id);
                }
                HirStableFunctionOrigin::ModulePrivate(origin) => {
                    if self
                        .module
                        .function_ids_by_private_origin
                        .contains_key(&origin)
                    {
                        return Err(CompilerError::compiler_error(format!(
                            "HIR function-origin lowering received duplicate private origin {:?}",
                            origin
                        )));
                    }
                    self.module
                        .function_ids_by_private_origin
                        .insert(origin, function.id);
                }
            }
        }

        // Reject any concrete origin seed that no lowered function consumed. An unmatched seed
        // means a public callable declaration did not lower to local HIR, which is an internal
        // invariant failure rather than a silent deferral to public-interface finalization.
        self.function_origin_lookup.validate_all_seeds_consumed()?;

        if let Some(start_function) = self.module.start_function {
            self.module
                .function_origins
                .insert(start_function, HirFunctionOrigin::EntryStart);
        }

        Ok(())
    }

    pub(super) fn resolve_doc_fragments(&mut self, ast: &Ast) -> Result<(), CompilerError> {
        self.extracted_metadata.doc_fragments.clear();

        for fragment in &ast.doc_fragments {
            let kind = match fragment.kind {
                AstDocFragmentKind::Doc => ModuleDocFragmentKind::Doc,
            };

            self.extracted_metadata
                .doc_fragments
                .push(ModuleDocFragment {
                    kind,
                    rendered_text: self.string_table.resolve(fragment.value).to_owned(),
                    location: fragment.location.to_owned(),
                });
        }

        Ok(())
    }
}

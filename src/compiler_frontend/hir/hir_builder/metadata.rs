//! Metadata passes for HIR module construction.
//!
//! WHAT: fills non-CFG module metadata after declarations and constants have
//! been prepared.
//! WHY: function origins are executable HIR metadata consumed by builders and later validation.
//! Resolved documentation fragments are non-HIR compiler metadata extracted into the lowering
//! metadata result boundary, not stored on `HirModule`.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::AstDocFragmentKind;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::pending_function_failure_facts;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ImplicitFailureContributor, ImplicitFailureSource,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::failure_facts::{
    HirBuiltinFailureBoundary, HirBuiltinFailureContributor, HirBuiltinFailureSource,
    HirFunctionFailureFacts,
};
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
                .consume_origin_for(&function_path)
            else {
                continue;
            };

            match origin {
                HirStableFunctionOrigin::Public(origin) => {
                    let facts = self
                        .module
                        .function_failure_facts
                        .get_mut(&function.id)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "HIR function-origin lowering is missing projected failure facts",
                            )
                        })?;
                    if facts.boundary == HirBuiltinFailureBoundary::InferPrivate {
                        facts.boundary = HirBuiltinFailureBoundary::ExportedNoSlot;
                    }
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

    /// Freeze expression-owned failure facts while the completed AST is still available.
    /// Exact exportedness is joined by origin assignment after this body projection.
    pub(super) fn project_function_failure_facts(&mut self, ast: &Ast) -> Result<(), CompilerError> {
        let builtin_error_type = self
            .type_environment
            .type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                CanonicalBuiltinType::Error,
            ));
        // Completed AST contains owned runtime handoffs, never unresolved TIR references.
        let template_ir_store = TemplateIrStore::new();

        for node in &ast.nodes {
            let NodeKind::Function(path, signature, body) = &node.kind else {
                continue;
            };
            let function_id = self.resolve_function_id_or_error(path, &node.span)?;
            let boundary = match signature.error_return_type_id() {
                _ if self.module.start_function == Some(function_id) => {
                    HirBuiltinFailureBoundary::BuiltinErrorSlot
                }
                Some(error_type) if Some(error_type) == builtin_error_type => {
                    HirBuiltinFailureBoundary::BuiltinErrorSlot
                }
                Some(error_type) => HirBuiltinFailureBoundary::CustomErrorSlot(error_type),
                None => HirBuiltinFailureBoundary::InferPrivate,
            };
            let pending = pending_function_failure_facts(body, &template_ir_store)?;
            let contributors = self.project_failure_contributors(pending.body.implicit)?;
            let assertion_message_calls =
                self.project_failure_contributors(pending.assertion_message_calls)?;
            self.module.function_failure_facts.insert(
                function_id,
                HirFunctionFailureFacts {
                    span: node.span,
                    boundary,
                    contributors,
                    assertion_message_calls,
                },
            );
        }

        Ok(())
    }

    fn project_failure_contributors(
        &self,
        pending: Vec<ImplicitFailureContributor>,
    ) -> Result<Vec<HirBuiltinFailureContributor>, CompilerError> {
        let mut contributors = Vec::with_capacity(pending.len());
        for contributor in pending {
            let source = match contributor.source {
                ImplicitFailureSource::NumericOperation => HirBuiltinFailureSource::NumericOperation,
                ImplicitFailureSource::PrivateCall(path) => HirBuiltinFailureSource::Call(
                    self.resolve_call_target_or_error(&path, &contributor.span)?,
                ),
            };
            contributors.push(HirBuiltinFailureContributor {
                source,
                span: contributor.span,
                codes: contributor.codes,
            });
        }
        Ok(contributors)
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
                    span: fragment.span,
                });
        }

        Ok(())
    }
}

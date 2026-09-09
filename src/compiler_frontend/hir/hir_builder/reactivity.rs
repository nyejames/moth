//! Reactivity metadata lowering for HIR construction.
//!
//! WHAT: converts AST-owned reactive source/template facts into HIR side-table records.
//! WHY: HIR should preserve backend-facing dependency metadata without reparsing template
//! directives, changing expression types, or adding backend render-plan nodes.

use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ReactiveSource, ReactiveSourceKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reactivity::{
    HirReactiveSource, HirReactiveSourceKind, HirReactiveTemplate, HirReactiveTemplateDependency,
    HirReactiveTemplateParameterDependency, ReactiveSourceId, ReactiveTemplateId,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(in crate::compiler_frontend::hir) fn bind_reactive_source_for_local(
        &mut self,
        local_id: LocalId,
        source: &ReactiveSource,
        type_id: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<ReactiveSourceId, CompilerError> {
        let source = HirReactiveSource {
            id: ReactiveSourceId(0),
            local_id,
            path: source.path.clone(),
            kind: hir_reactive_source_kind(source.kind),
            type_id,
            span: *span,
        };

        Ok(self.side_table.bind_reactive_source(source))
    }

    pub(in crate::compiler_frontend::hir) fn bind_reactive_metadata_for_expression(
        &mut self,
        expression: &Expression,
        value: &HirExpression,
    ) -> Result<(), CompilerError> {
        if let Some(source) = &expression.reactive_source
            && let HirExpressionKind::Load(HirPlace::Local(local_id)) = &value.kind
            && self
                .side_table
                .reactive_source_id_for_local(*local_id)
                .is_none()
        {
            self.bind_reactive_source_for_local(*local_id, source, value.ty, &expression.span)?;
        }

        if let Some(metadata) = &expression.reactive_template
            && value.ty == self.type_environment.builtins().string
        {
            self.bind_reactive_template_metadata_for_value(value.id, metadata, &expression.span)?;
        }

        Ok(())
    }

    pub(super) fn bind_reactive_template_metadata_for_value(
        &mut self,
        value_id: HirValueId,
        metadata: &ReactiveTemplateMetadata,
        span: &Option<SourceSpan>,
    ) -> Result<ReactiveTemplateId, CompilerError> {
        let mut dependencies = Vec::with_capacity(metadata.subscriptions.len());
        for subscription in &metadata.subscriptions {
            let source =
                self.resolve_reactive_source_id(&subscription.source, &subscription.span)?;
            let type_id = self.lower_type_id(subscription.type_id, &subscription.span)?;
            dependencies.push(HirReactiveTemplateDependency {
                source,
                type_id,
                span: subscription.span,
            });
        }

        let mut template_value_parameters =
            Vec::with_capacity(metadata.template_value_parameters.len());
        for dependency in &metadata.template_value_parameters {
            let Some(parameter) = self.locals_by_name.get(&dependency.parameter).copied() else {
                return_hir_transformation_error!(
                    format!(
                        "Reactive template parameter '{}' was not registered as a HIR local",
                        self.symbol_name_for_diagnostics(&dependency.parameter)
                    ),
                    dependency.span
                );
            };

            template_value_parameters.push(HirReactiveTemplateParameterDependency {
                parameter,
                span: dependency.span,
            });
        }

        let template = HirReactiveTemplate {
            id: ReactiveTemplateId(0),
            value_id,
            dependencies,
            template_value_parameters,
            template_backed: metadata.template_backed,
            span: *span,
        };

        Ok(self.side_table.bind_reactive_template(template))
    }

    fn resolve_reactive_source_id(
        &self,
        source: &ReactiveSource,
        span: &Option<SourceSpan>,
    ) -> Result<ReactiveSourceId, CompilerError> {
        if let Some(source_id) = self.side_table.reactive_source_id_for_path(&source.path) {
            return Ok(source_id);
        }

        if let Some(local_id) = self.locals_by_name.get(&source.path)
            && let Some(source_id) = self.side_table.reactive_source_id_for_local(*local_id)
        {
            return Ok(source_id);
        }

        return_hir_transformation_error!(
            format!(
                "Reactive template dependency '{}' did not resolve to a HIR reactive source",
                self.symbol_name_for_diagnostics(&source.path)
            ),
            *span
        )
    }
}

fn hir_reactive_source_kind(kind: ReactiveSourceKind) -> HirReactiveSourceKind {
    match kind {
        ReactiveSourceKind::Declaration => HirReactiveSourceKind::Declaration,
        ReactiveSourceKind::Parameter => HirReactiveSourceKind::Parameter,
    }
}

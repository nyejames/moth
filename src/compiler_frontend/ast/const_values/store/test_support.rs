//! Fixture construction for the module-local folded-value store.
//!
//! WHAT: builds a [`ConstValueStore`] directly from AST [`Declaration`]s, bypassing the
//! declaration table and the real TIR template projection.
//! WHY: HIR and public-interface unit fixtures need a populated store without running AST
//! finalization. Keeping these constructors here rather than in `store.rs` holds the production
//! file to the shapes the compiler actually builds, per the style guide's rule that production
//! files must not grow test-only constructors or mutators.

use super::{
    ConstStringValue, ConstTemplateValue, ConstValueKind, ConstValueRow, ConstValueStore,
    ConstValueStoreError,
};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::folded_value::{PublicConstTemplate, PublicConstTemplateKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

impl ConstValueStore {
    /// Build a store for HIR and public-interface unit fixtures.
    pub(crate) fn from_test_declarations(
        declarations: Vec<Declaration>,
        type_environment: &TypeEnvironment,
    ) -> Result<Self, CompilerError> {
        let mut store = Self::default();
        for declaration in declarations {
            store.try_insert_test_declaration(declaration, type_environment)?;
        }
        Ok(store)
    }

    /// Append one declaration to a test-owned store.
    pub(crate) fn insert_test_declaration(
        &mut self,
        declaration: Declaration,
        type_environment: &TypeEnvironment,
    ) {
        self.try_insert_test_declaration(declaration, type_environment)
            .expect("test declaration must be a supported folded store value");
    }

    pub(crate) fn try_insert_test_declaration(
        &mut self,
        declaration: Declaration,
        type_environment: &TypeEnvironment,
    ) -> Result<(), CompilerError> {
        let mut template_builder =
            |_: Option<&InternedPath>,
             _: &crate::compiler_frontend::ast::templates::template::Template|
             -> Result<ConstTemplateValue, ConstValueStoreError> {
                Ok(test_wrapper_template_value(None))
            };
        self.insert_test_declaration_with_builder(
            declaration,
            &mut template_builder,
            type_environment,
        )
    }

    /// Build a store whose template expressions fold to one fixed folded-string value.
    ///
    /// WHAT: supplies the [`ConstTemplateValue`] a real TIR projection would hand the store,
    /// including piece-bearing folds no producer creates yet.
    /// WHY: store-level tests must exercise the template payload exactly as finalization
    /// supplies it rather than through a reconstructed string row.
    pub(crate) fn from_test_template_folds(
        declarations: Vec<Declaration>,
        folded: ConstStringValue,
        type_environment: &TypeEnvironment,
    ) -> Result<Self, CompilerError> {
        let mut store = Self::default();
        for declaration in declarations {
            let folded = folded.clone();
            let mut template_builder =
                |_: Option<&InternedPath>,
                 _: &crate::compiler_frontend::ast::templates::template::Template|
                 -> Result<ConstTemplateValue, ConstValueStoreError> {
                    Ok(ConstTemplateValue::Folded {
                        value: folded.clone(),
                        provenance: SyntheticInterfaceProvenance::empty(),
                    })
                };
            store.insert_test_declaration_with_builder(
                declaration,
                &mut template_builder,
                type_environment,
            )?;
        }
        Ok(store)
    }

    /// Append one template declaration whose fold materializes to a fixed folded value.
    ///
    /// WHAT: supplies the [`ConstTemplateValue::Public`] wrapper shape production projection
    /// hands the store, with the folded result fixed, so the row keeps the real
    /// `hir_visible` wrapper footprint and reaches the folded `Template` payload arm.
    /// WHY: HIR module-constant fixtures need a folded template row without running TIR
    /// finalization, and `from_test_template_folds` above only stores `Folded` string rows
    /// that bypass the `Template` payload entirely.
    pub(crate) fn insert_test_template_fold(
        &mut self,
        declaration: Declaration,
        folded: ConstStringValue,
        type_environment: &TypeEnvironment,
    ) {
        let mut template_builder =
            |_: Option<&InternedPath>,
             _: &crate::compiler_frontend::ast::templates::template::Template|
             -> Result<ConstTemplateValue, ConstValueStoreError> {
                Ok(test_wrapper_template_value(Some(folded.clone())))
            };
        self.insert_test_declaration_with_builder(
            declaration,
            &mut template_builder,
            type_environment,
        )
        .expect("test template-fold declaration must be a supported folded store value");
    }

    fn insert_test_declaration_with_builder(
        &mut self,
        declaration: Declaration,
        template_builder: &mut impl FnMut(
            Option<&InternedPath>,
            &crate::compiler_frontend::ast::templates::template::Template,
        ) -> Result<ConstTemplateValue, ConstValueStoreError>,
        type_environment: &TypeEnvironment,
    ) -> Result<(), CompilerError> {
        let value = self
            .insert_expression(
                &declaration.value,
                Some(&declaration.id),
                type_environment,
                template_builder,
            )
            .map_err(|error| match error {
                ConstValueStoreError::Infrastructure(error) => *error,
                ConstValueStoreError::Diagnostic(diagnostic) => CompilerError::compiler_error(
                    format!("test constant store insertion received a diagnostic: {diagnostic:?}"),
                ),
            })?;
        let path = declaration.id;
        if self.values_by_path.insert(path.clone(), value).is_some() {
            return Err(CompilerError::compiler_error(
                "two finalized module constants share the defining path",
            ));
        }
        self.rows.push(ConstValueRow { path, value });
        Ok(())
    }
}

fn test_wrapper_template_value(folded: Option<ConstStringValue>) -> ConstTemplateValue {
    ConstTemplateValue::Public {
        template: PublicConstTemplate {
            kind: PublicConstTemplateKind::Wrapper,
            pieces: Vec::new(),
            conditional_child_wrappers: Vec::new(),
        },
        kind: ConstValueKind::TemplateWrapper,
        hir_visible: true,
        folded,
        provenance: SyntheticInterfaceProvenance::empty(),
    }
}

//! Declaration-semantic classification regression tests.
//!
//! WHAT: verifies that classification preserves malformed retained template authority.
//! WHY: environment finalisation must report compiler infrastructure failures through the
//! infrastructure message lane instead of re-rendering them as authored diagnostics.

use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use super::{DeclarationSemanticTable, TopLevelDeclarationTable};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrStore, TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn declaration_semantics_preserves_missing_template_authority() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let declaration = {
        let value = expression_builder
            .template(
                Template {
                    tir_reference: TemplateTirReference {
                        root: TemplateIrId::new(99),
                        phase: TemplateTirPhase::Composed,
                        context: TemplateViewContext::default(),
                    },
                    span: None,
                },
                ValueMode::ImmutableOwned,
            )
            .expect("template occurrence fits")
            .id();
        Declaration {
            id: path_fork
                .try_intern_portable_path("value", &mut string_table)
                .expect("test path fits"),
            type_id: crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
            type_syntax: None,
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Initializer(value),
            binding_span: None,
            config_qualifier: None,
        }
    };
    let table = TopLevelDeclarationTable::new(vec![declaration], &path_fork);
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let error = DeclarationSemanticTable::from_environment(
        &table,
        &FxHashMap::default(),
        &FxHashMap::default(),
        &TypeEnvironment::new(),
        &store,
        &expression_builder,
        &ConstValueStore::default(),
    )
    .expect_err("missing declaration template authority must fail classification");

    assert!(matches!(error, TemplateError::Infrastructure(_)));
}

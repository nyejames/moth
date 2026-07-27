//! HIR Display
//!
//! Responsible for providing a way to get location and variable name information back from HIR.
//!
//! This will be used to help the rest of the HIR and borrow checker stages to create and return useful errors and warnings.
//! (CompilerMessages)
//! It will also enable printing out Hir structures for easy debugging also.

#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::datatypes::definitions::{
    BuiltinTypeDefinition, ChoiceTypeDefinition, ConstructedTypeDefinition, FunctionTypeDefinition,
    StructTypeDefinition, TypeDefinition,
};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::datatypes::display::display_type;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::datatypes::ids::TypeId;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, TypeConstructor,
};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::external_packages::CallTarget;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
#[cfg(test)]
use crate::compiler_frontend::hir::expressions::FallibleCarrierVariant;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, ValueKind,
};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::functions::HirFunction;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::ids::ChoiceId;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId, StructId,
};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOp;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::numeric::{HirNumericOperands, NumericFailureMode};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::places::HirPlace;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::hir::terminators::HirTerminator;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::symbols::string_interning::StringTable;
#[cfg(any(test, feature = "show_hir"))]
use std::fmt::Write as _;
use std::fmt::{Display, Formatter, Result as FmtResult};

#[cfg(any(test, feature = "show_hir"))]
const MAX_TYPE_RENDER_DEPTH: usize = 24;

#[cfg(any(test, feature = "show_hir"))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct HirDisplayOptions {
    pub include_ids: bool,
    pub include_types: bool,
    pub include_value_kinds: bool,
    pub include_regions: bool,
    pub multiline_match_arms: bool,
}

#[cfg(any(test, feature = "show_hir"))]
impl Default for HirDisplayOptions {
    fn default() -> Self {
        Self {
            include_ids: true,
            include_types: true,
            include_value_kinds: false,
            include_regions: true,
            multiline_match_arms: true,
        }
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[derive(Clone, Copy)]
pub(crate) struct HirDisplayContext<'a> {
    string_table: &'a StringTable,
    side_table: Option<&'a HirSideTable>,
    type_environment: Option<&'a TypeEnvironment>,
    options: HirDisplayOptions,
}

#[cfg(any(test, feature = "show_hir"))]
impl<'a> HirDisplayContext<'a> {
    pub(crate) fn new(string_table: &'a StringTable) -> Self {
        Self {
            string_table,
            side_table: None,
            type_environment: None,
            options: HirDisplayOptions::default(),
        }
    }

    #[cfg(feature = "show_hir")]
    pub(crate) fn with_side_table(mut self, side_table: &'a HirSideTable) -> Self {
        self.side_table = Some(side_table);
        self
    }

    #[cfg(feature = "show_hir")]
    pub(crate) fn with_type_environment(mut self, type_environment: &'a TypeEnvironment) -> Self {
        self.type_environment = Some(type_environment);
        self
    }

    #[cfg(any(test, feature = "show_hir"))]
    #[allow(dead_code)] // show_hir builds keep this test helper for focused display rendering.
    pub(crate) fn with_options(mut self, options: HirDisplayOptions) -> Self {
        self.options = options;
        self
    }

    pub(crate) fn render_module(&self, module: &HirModule) -> String {
        let mut out = String::with_capacity(
            module.blocks.len() * 160 + module.functions.len() * 64 + module.structs.len() * 64,
        );

        out.push_str("hir_module {\n");
        let start_function = module
            .start_function
            .map(|function_id| self.function_label(function_id))
            .unwrap_or_else(|| String::from("none"));
        let _ = writeln!(out, "  start_function: {start_function}");

        let _ = writeln!(out, "  regions: {}", module.regions.len());

        out.push_str("  functions:\n");
        if module.functions.is_empty() {
            out.push_str("    (none)\n");
        } else {
            for function in &module.functions {
                self.push_indented_line(&mut out, 4, &self.render_function(function));
            }
        }

        out.push_str("  structs:\n");
        if module.structs.is_empty() {
            out.push_str("    (none)\n");
        } else {
            for hir_struct in &module.structs {
                self.push_indented_line(&mut out, 4, &self.render_struct(hir_struct));
            }
        }

        out.push_str("  blocks:\n");
        if module.blocks.is_empty() {
            out.push_str("    (none)\n");
        } else {
            for block in &module.blocks {
                let block_rendered = self.render_block(block);
                self.push_indented_multiline(&mut out, 4, &block_rendered);
            }
        }

        out.push('}');
        out
    }

    pub(crate) fn render_struct(&self, hir_struct: &HirStruct) -> String {
        let mut out = String::new();
        let _ = write!(out, "{} {{ ", self.struct_label(hir_struct.id));

        for (idx, field) in hir_struct.fields.iter().enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(&self.render_field(field));
        }

        out.push_str(" }");
        out
    }

    pub(crate) fn render_field(&self, field: &HirField) -> String {
        if self.options.include_types {
            format!(
                "{}: {}",
                self.field_label(field.id),
                self.type_label(field.ty)
            )
        } else {
            self.field_label(field.id)
        }
    }

    pub(crate) fn render_function(&self, function: &HirFunction) -> String {
        let mut out = String::new();
        let params = function
            .params
            .iter()
            .map(|param| self.local_label(*param))
            .collect::<Vec<_>>()
            .join(", ");

        let _ = write!(out, "{}({})", self.function_label(function.id), params);

        if self.options.include_types {
            let _ = write!(out, " -> {}", self.type_label(function.return_type));
        }

        let _ = write!(out, " [entry: {}]", self.block_label(function.entry));
        out
    }

    pub(crate) fn render_block(&self, block: &HirBlock) -> String {
        let mut out = String::new();
        let _ = write!(out, "{} ", self.block_label(block.id));

        if self.options.include_regions {
            let _ = write!(out, "[region: {}]", self.region_label(block.region));
        }

        out.push('\n');

        if block.locals.is_empty() {
            out.push_str("  locals: (none)\n");
        } else {
            out.push_str("  locals:\n");
            for local in &block.locals {
                let rendered = self.render_local(local);
                self.push_indented_line(&mut out, 4, &rendered);
            }
        }

        if block.statements.is_empty() {
            out.push_str("  statements: (none)\n");
        } else {
            out.push_str("  statements:\n");
            for statement in &block.statements {
                let rendered = self.render_statement(statement);
                self.push_indented_line(&mut out, 4, &rendered);
            }
        }

        out.push_str("  terminator: ");
        out.push_str(&self.render_terminator(&block.terminator));
        out.push('\n');

        out
    }

    pub(crate) fn render_local(&self, local: &HirLocal) -> String {
        let mut out = String::new();

        if local.mutable {
            out.push_str("mut ");
        }

        out.push_str(&self.local_label(local.id));

        if self.options.include_types {
            let _ = write!(out, ": {}", self.type_label(local.ty));
        }

        if self.options.include_regions {
            let _ = write!(out, " [{}]", self.region_label(local.region));
        }

        out
    }

    pub(crate) fn render_statement(&self, statement: &HirStatement) -> String {
        let mut out = String::new();

        if self.options.include_ids {
            let _ = write!(out, "[{}] ", self.node_label(statement.id));
        }

        out.push_str(&self.render_statement_kind(&statement.kind));
        out
    }

    pub(crate) fn render_statement_kind(&self, kind: &HirStatementKind) -> String {
        match kind {
            HirStatementKind::Assign { target, value } => {
                format!(
                    "{} = {}",
                    self.render_place(target),
                    self.render_expression(value)
                )
            }
            HirStatementKind::Call {
                target,
                args,
                result,
            } => {
                let mut out = String::new();

                if let Some(local) = result {
                    let _ = write!(out, "{} = ", self.local_label(*local));
                }

                let args_rendered = args
                    .iter()
                    .map(|arg| self.render_expression(arg))
                    .collect::<Vec<_>>()
                    .join(", ");

                let _ = write!(
                    out,
                    "call {}({})",
                    self.render_call_target(target),
                    args_rendered
                );

                out
            }
            HirStatementKind::Expr(expr) => self.render_expression(expr),
            HirStatementKind::Drop(local) => format!("drop {}", self.local_label(*local)),
            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                format!(
                    "push_fragment {} <- {}",
                    self.local_label(*vec_local),
                    self.render_expression(value)
                )
            }

            HirStatementKind::CastOp {
                policy,
                source,
                result,
            } => {
                let mut out = String::new();
                if let Some(local) = result {
                    let _ = write!(out, "{} = ", self.local_label(*local));
                }
                let _ = write!(out, "cast_{:?}({})", policy, self.render_expression(source));
                out
            }

            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => {
                let mut out = String::new();

                if let Some(local) = result {
                    let _ = write!(out, "{} = ", self.local_label(*local));
                }

                let _ = write!(
                    out,
                    "map_{}({}{}",
                    op.source_name(),
                    if op.requires_mutable_receiver() {
                        "~"
                    } else {
                        ""
                    },
                    self.render_expression(receiver)
                );

                for arg in args {
                    let _ = write!(out, ", {}", self.render_expression(arg));
                }

                out.push(')');
                out
            }

            HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } => {
                let mut out = String::new();

                let _ = write!(out, "{} = ", self.local_label(*result));
                let _ = write!(
                    out,
                    "numeric_{}_{}(",
                    op.source_name(),
                    match failure_mode {
                        NumericFailureMode::ReturnError => "err",
                        NumericFailureMode::Trap => "trap",
                    }
                );

                match operands {
                    HirNumericOperands::Unary { operand } => {
                        let _ = write!(out, "{}", self.render_expression(operand));
                    }
                    HirNumericOperands::Binary { left, right } => {
                        let _ = write!(
                            out,
                            "{}, {}",
                            self.render_expression(left),
                            self.render_expression(right)
                        );
                    }
                }

                out.push(')');
                out
            }

            HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            } => {
                format!(
                    "{} = format_float_{}({})",
                    self.local_label(*result),
                    match failure_mode {
                        NumericFailureMode::ReturnError => "err",
                        NumericFailureMode::Trap => "trap",
                    },
                    self.render_expression(source)
                )
            }

            HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            } => {
                format!(
                    "{} = validate_float_{}({})",
                    self.local_label(*result),
                    match failure_mode {
                        NumericFailureMode::ReturnError => "err",
                        NumericFailureMode::Trap => "trap",
                    },
                    self.render_expression(source)
                )
            }
        }
    }

    pub(crate) fn render_terminator(&self, terminator: &HirTerminator) -> String {
        match terminator {
            HirTerminator::Jump { target, args } => {
                if args.is_empty() {
                    format!("jump {}", self.block_label(*target))
                } else {
                    let args_rendered = args
                        .iter()
                        .map(|arg| self.local_label(*arg))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("jump {}({})", self.block_label(*target), args_rendered)
                }
            }
            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                format!(
                    "if {} -> {} else {}",
                    self.render_expression(condition),
                    self.block_label(*then_block),
                    self.block_label(*else_block)
                )
            }
            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                format!(
                    "fallible {} -> ok {} else err {}",
                    self.render_expression(result),
                    self.block_label(*success_block),
                    self.block_label(*error_block)
                )
            }
            HirTerminator::Match { scrutinee, arms } => {
                if !self.options.multiline_match_arms {
                    let arms_rendered = arms
                        .iter()
                        .map(|arm| self.render_match_arm(arm))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return format!(
                        "match {} {{ {} }}",
                        self.render_expression(scrutinee),
                        arms_rendered
                    );
                }

                let mut out = String::new();
                let _ = writeln!(out, "match {} {{", self.render_expression(scrutinee));
                for arm in arms {
                    let _ = writeln!(out, "  {},", self.render_match_arm(arm));
                }
                out.push('}');
                out
            }
            HirTerminator::Break { target } => format!("break {}", self.block_label(*target)),
            HirTerminator::Continue { target } => {
                format!("continue {}", self.block_label(*target))
            }
            HirTerminator::Return(value) => format!("return {}", self.render_expression(value)),
            HirTerminator::ReturnSuccess(value) => {
                format!("return ok {}", self.render_expression(value))
            }
            HirTerminator::ReturnError(value) => {
                format!("return! {}", self.render_expression(value))
            }
            HirTerminator::Uninitialized => "uninitialized".to_owned(),
            HirTerminator::RuntimeFailure { message } => {
                format!("runtime_failure \"{}\"", message.escape_debug())
            }
            HirTerminator::AssertFailure { message } => match message {
                Some(msg) => format!("assert_failure \"{}\"", msg.escape_debug()),
                None => "assert_failure".to_owned(),
            },
        }
    }

    pub(crate) fn render_expression(&self, expr: &HirExpression) -> String {
        let mut out = String::new();

        if self.options.include_ids {
            let _ = write!(out, "[{}] ", self.value_label(expr.id));
        }

        out.push_str(&self.render_expression_kind(&expr.kind));

        if self.options.include_types {
            let _ = write!(out, " : {}", self.type_label(expr.ty));
        }

        if self.options.include_value_kinds {
            let _ = write!(out, " [{}]", self.value_kind_label(expr.value_kind));
        }

        out
    }

    pub(crate) fn render_expression_kind(&self, kind: &HirExpressionKind) -> String {
        match kind {
            HirExpressionKind::Int(value) => value.to_string(),
            HirExpressionKind::Float(value) => value.to_string(),
            HirExpressionKind::Bool(value) => value.to_string(),
            HirExpressionKind::Char(value) => format!("'{}'", value.escape_debug()),
            HirExpressionKind::StringLiteral(value) => {
                format!("\"{}\"", value.escape_debug())
            }
            HirExpressionKind::Load(place) => self.render_place(place),
            HirExpressionKind::Copy(place) => format!("copy {}", self.render_place(place)),
            HirExpressionKind::BinOp { left, op, right } => format!(
                "({} {} {})",
                self.render_expression(left),
                op,
                self.render_expression(right)
            ),
            HirExpressionKind::UnaryOp { op, operand } => {
                format!("({}{})", op, self.render_expression(operand))
            }
            HirExpressionKind::StructConstruct { struct_id, fields } => {
                let mut out = String::new();
                let _ = write!(out, "{} {{ ", self.struct_label(*struct_id));

                for (idx, (field_id, expr)) in fields.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(
                        out,
                        "{}: {}",
                        self.field_label(*field_id),
                        self.render_expression(expr)
                    );
                }

                out.push_str(" }");
                out
            }
            HirExpressionKind::MapLiteral(entries) => {
                let joined = entries
                    .iter()
                    .map(|entry| {
                        format!(
                            "{} = {}",
                            self.render_expression(&entry.key),
                            self.render_expression(&entry.value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{joined}}}")
            }
            HirExpressionKind::Collection(elements) => {
                let joined = elements
                    .iter()
                    .map(|element| self.render_expression(element))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{joined}]")
            }
            HirExpressionKind::Range { start, end } => {
                format!(
                    "{start}..{end}",
                    start = self.render_expression(start),
                    end = self.render_expression(end)
                )
            }
            HirExpressionKind::TupleConstruct { elements } => {
                if elements.is_empty() {
                    return "()".to_owned();
                }

                let joined = elements
                    .iter()
                    .map(|element| self.render_expression(element))
                    .collect::<Vec<_>>()
                    .join(", ");

                if elements.len() == 1 {
                    format!("({joined},)")
                } else {
                    format!("({joined})")
                }
            }
            HirExpressionKind::TupleGet { tuple, index } => {
                format!("tuple_get({}, {})", self.render_expression(tuple), index)
            }
            HirExpressionKind::FallibleUnwrapSuccess { result } => {
                format!("result_unwrap_ok({})", self.render_expression(result))
            }
            HirExpressionKind::FallibleUnwrapError { result } => {
                format!("result_unwrap_err({})", self.render_expression(result))
            }
            HirExpressionKind::Cast { source, policy } => {
                format!("cast_{:?}({})", policy, self.render_expression(source))
            }
            HirExpressionKind::VariantConstruct {
                carrier,
                variant_index,
                fields,
            } => {
                let carrier_label = self.variant_carrier_label(carrier);
                let fields_str = if fields.is_empty() {
                    String::new()
                } else {
                    let rendered: Vec<String> = fields
                        .iter()
                        .map(|field| {
                            let name_str = field
                                .name
                                .map(|n| self.string_table.resolve(n))
                                .unwrap_or("?");
                            format!("{}: {}", name_str, self.render_expression(&field.value))
                        })
                        .collect();
                    format!(", fields=[{}]", rendered.join(", "))
                };
                format!("variant_construct({carrier_label}, tag={variant_index}){fields_str}")
            }
            HirExpressionKind::VariantPayloadGet {
                carrier,
                source,
                variant_index,
                field_index,
            } => {
                let carrier_label = self.variant_carrier_label(carrier);
                format!(
                    "variant_payload_get({}, tag={}, field={}) from {}",
                    carrier_label,
                    variant_index,
                    field_index,
                    self.render_expression(source)
                )
            }
        }
    }

    pub(crate) fn render_place(&self, place: &HirPlace) -> String {
        match place {
            HirPlace::Local(local_id) => self.local_label(*local_id),
            HirPlace::Field { base, field } => {
                format!("{}.{}", self.render_place(base), self.field_label(*field))
            }
            HirPlace::Index { base, index } => {
                format!(
                    "{}[{}]",
                    self.render_place(base),
                    self.render_expression(index)
                )
            }
        }
    }

    fn render_relational_pattern_op(&self, op: HirRelationalPatternOp) -> &'static str {
        match op {
            HirRelationalPatternOp::LessThan => "<",
            HirRelationalPatternOp::LessThanOrEqual => "<=",
            HirRelationalPatternOp::GreaterThan => ">",
            HirRelationalPatternOp::GreaterThanOrEqual => ">=",
        }
    }

    pub(crate) fn render_pattern(&self, pattern: &HirPattern) -> String {
        match pattern {
            HirPattern::Literal(expr) => self.render_expression(expr),
            HirPattern::OptionNone => "none".to_owned(),
            HirPattern::OptionValue { value } => {
                format!("some({})", self.render_expression(value))
            }
            HirPattern::OptionRelational { op, value } => {
                format!(
                    "some({} {})",
                    self.render_relational_pattern_op(*op),
                    self.render_expression(value)
                )
            }
            HirPattern::Wildcard => "_".to_owned(),
            HirPattern::Relational { op, value } => {
                format!(
                    "{} {}",
                    self.render_relational_pattern_op(*op),
                    self.render_expression(value)
                )
            }
            HirPattern::Capture => "capture".to_owned(),
            HirPattern::OptionPresent => "|capture|".to_owned(),
            HirPattern::ChoiceVariant {
                choice_id,
                variant_index,
            } => {
                format!(
                    "choice_pat(choice={}, tag={})",
                    self.choice_label(*choice_id),
                    variant_index
                )
            }
        }
    }

    pub(crate) fn render_match_arm(&self, arm: &HirMatchArm) -> String {
        let mut out = String::new();
        out.push_str(&self.render_pattern(&arm.pattern));

        if let Some(guard) = &arm.guard {
            let _ = write!(out, " if {}", self.render_expression(guard));
        }

        let _ = write!(out, " => {}", self.block_label(arm.body));
        out
    }

    fn render_call_target(&self, target: &CallTarget) -> String {
        match target {
            CallTarget::Local(function_id) => self.function_label(*function_id),
            CallTarget::CrossModule(origin) => {
                format!("imported {}", origin.defining_name())
            }
            CallTarget::External(id) => id.name().to_owned(),
        }
    }

    fn value_kind_label(&self, value_kind: ValueKind) -> &'static str {
        match value_kind {
            ValueKind::Place => "place",
            ValueKind::RValue => "rvalue",
            ValueKind::Const => "const",
        }
    }

    fn type_label(&self, ty: TypeId) -> String {
        let Some(type_environment) = self.type_environment else {
            return format!("t{}", ty.0);
        };

        self.render_type_with_environment(type_environment, ty, 0)
    }

    fn render_type_with_environment(
        &self,
        type_environment: &TypeEnvironment,
        ty: TypeId,
        depth: usize,
    ) -> String {
        if depth >= MAX_TYPE_RENDER_DEPTH {
            return format!("t{}", ty.0);
        }

        let Some(definition) = type_environment.get(ty) else {
            return format!("t{}", ty.0);
        };

        match definition {
            TypeDefinition::Builtin(BuiltinTypeDefinition { key }) => match key {
                BuiltinTypeKey::Bool => "Bool".to_owned(),
                BuiltinTypeKey::Int => "Int".to_owned(),
                BuiltinTypeKey::Float => "Float".to_owned(),
                // Decimal is intentionally inactive in the Alpha surface.
                BuiltinTypeKey::Decimal => "Decimal".to_owned(),
                BuiltinTypeKey::Char => "Char".to_owned(),
                BuiltinTypeKey::String => "String".to_owned(),
                BuiltinTypeKey::Range => "Range".to_owned(),
                BuiltinTypeKey::None => "()".to_owned(),
            },
            TypeDefinition::Struct(StructTypeDefinition { path, .. }) => {
                path.to_string(self.string_table)
            }
            TypeDefinition::Choice(ChoiceTypeDefinition { path, .. }) => {
                format!("Choice({})", path.to_string(self.string_table))
            }
            TypeDefinition::Function(FunctionTypeDefinition {
                parameters,
                returns,
                error_return,
                ..
            }) => {
                let params = parameters
                    .iter()
                    .map(|param| {
                        self.render_type_with_environment(
                            type_environment,
                            param.type_id,
                            depth + 1,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                let returns = if returns.is_empty() {
                    "()".to_owned()
                } else if returns.len() == 1 {
                    self.render_type_with_environment(type_environment, returns[0], depth + 1)
                } else {
                    let joined = returns
                        .iter()
                        .map(|ret| {
                            self.render_type_with_environment(type_environment, *ret, depth + 1)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({joined})")
                };

                let error = error_return.map(|err| {
                    format!(
                        " throws {}",
                        self.render_type_with_environment(type_environment, err, depth + 1)
                    )
                });

                match error {
                    Some(error) => format!("fn({params})->{returns}{error}"),
                    None => format!("fn({params})->{returns}"),
                }
            }
            TypeDefinition::Constructed(ConstructedTypeDefinition {
                constructor,
                arguments,
            }) => match constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple) => {
                    if arguments.is_empty() {
                        return "()".to_owned();
                    }
                    let joined = arguments
                        .iter()
                        .map(|field| {
                            self.render_type_with_environment(type_environment, *field, depth + 1)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({joined})")
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { .. }) => {
                    display_type(ty, type_environment, self.string_table)
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
                    display_type(ty, type_environment, self.string_table)
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
                    let inner = arguments.first().map_or_else(
                        || "?".to_owned(),
                        |arg| self.render_type_with_environment(type_environment, *arg, depth + 1),
                    );
                    format!("Option<{inner}>")
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier) => {
                    let success = arguments.first().map_or_else(
                        || "?".to_owned(),
                        |arg| self.render_type_with_environment(type_environment, *arg, depth + 1),
                    );
                    let error = arguments.get(1).map_or_else(
                        || "?".to_owned(),
                        |arg| self.render_type_with_environment(type_environment, *arg, depth + 1),
                    );
                    format!("FallibleCarrier<{success}, {error}>")
                }
            },
            TypeDefinition::External(external_def) => {
                format!("External({})", external_def.type_id.0)
            }
            TypeDefinition::GenericParameter(param) => {
                format!("GenericParam({})", param.id.0)
            }
            TypeDefinition::GenericInstance(instance) => {
                let args = instance
                    .arguments
                    .iter()
                    .map(|arg| self.render_type_with_environment(type_environment, *arg, depth + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                let name = type_environment
                    .nominal_path_by_id(instance.base)
                    .map(|path| path.to_string(self.string_table))
                    .unwrap_or_else(|| format!("GenericInstance({})", instance.base.0));
                if args.is_empty() {
                    name
                } else {
                    format!("{name}<{args}>")
                }
            }
        }
    }

    fn local_label(&self, local_id: LocalId) -> String {
        if let Some(name) = self
            .side_table
            .and_then(|side| side.resolve_local_name(local_id, self.string_table))
        {
            return name.to_owned();
        }

        format!("l{}", local_id.0)
    }

    fn function_label(&self, function_id: FunctionId) -> String {
        if let Some(name) = self
            .side_table
            .and_then(|side| side.resolve_function_name(function_id, self.string_table))
        {
            return name.to_owned();
        }

        format!("fn{}", function_id.0)
    }

    fn struct_label(&self, struct_id: StructId) -> String {
        if let Some(name) = self
            .side_table
            .and_then(|side| side.display_struct_name(struct_id, self.string_table))
        {
            return name;
        }

        format!("struct{}", struct_id.0)
    }

    fn choice_label(&self, choice_id: ChoiceId) -> String {
        if let Some(name) = self
            .side_table
            .and_then(|side| side.display_choice_name(choice_id, self.string_table))
        {
            return name;
        }

        format!("choice{}", choice_id.0)
    }

    fn field_label(&self, field_id: FieldId) -> String {
        if let Some(name) = self
            .side_table
            .and_then(|side| side.resolve_field_name(field_id, self.string_table))
        {
            return name.to_owned();
        }

        format!("field{}", field_id.0)
    }

    fn block_label(&self, block_id: BlockId) -> String {
        format!("bb{}", block_id.0)
    }

    fn node_label(&self, node_id: HirNodeId) -> String {
        format!("n{}", node_id.0)
    }

    fn value_label(&self, value_id: HirValueId) -> String {
        format!("v{}", value_id.0)
    }

    fn region_label(&self, region_id: RegionId) -> String {
        format!("r{}", region_id.0)
    }

    fn variant_carrier_label(&self, carrier: &HirVariantCarrier) -> String {
        match carrier {
            #[cfg(any(test, feature = "show_hir"))]
            HirVariantCarrier::Choice { choice_id } => {
                format!("choice={}", self.choice_label(*choice_id))
            }
            #[cfg(not(any(test, feature = "show_hir")))]
            HirVariantCarrier::Choice { .. } => "choice".to_owned(),
            HirVariantCarrier::Option => "option".to_owned(),
            #[cfg(test)]
            HirVariantCarrier::Fallible => "result".to_owned(),
        }
    }

    fn push_indented_line(&self, out: &mut String, indent: usize, line: &str) {
        for _ in 0..indent {
            out.push(' ');
        }
        out.push_str(line);
        out.push('\n');
    }

    fn push_indented_multiline(&self, out: &mut String, indent: usize, text: &str) {
        for line in text.lines() {
            self.push_indented_line(out, indent, line);
        }
    }
}

// -----------------------------
//  Convenience Display Hooks
// -----------------------------

// Debug display helpers: gated to test builds and the "show_hir" feature.
// The `#[allow(dead_code)]` annotations are needed because rustc does not see
// calls made through the `hir_log!` macro as usages for dead-code analysis.
#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirModule {
    pub(crate) fn display_with_table(&self, string_table: &StringTable) -> String {
        HirDisplayContext::new(string_table).render_module(self)
    }

    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_module(self)
    }

    pub(crate) fn debug_string(&self, string_table: &StringTable) -> String {
        self.display_with_table(string_table)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirBlock {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_block(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirFunction {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_function(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirStruct {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_struct(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirStatement {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_statement(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirTerminator {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_terminator(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirExpression {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_expression(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirPlace {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_place(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirPattern {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_pattern(self)
    }
}

#[cfg(any(test, feature = "show_hir"))]
#[allow(dead_code)]
impl HirMatchArm {
    pub(crate) fn display_with_context(&self, display: &HirDisplayContext<'_>) -> String {
        display.render_match_arm(self)
    }
}

// ---------------------------
//  Simple Token Displays
// ---------------------------

macro_rules! impl_hir_id_display {
    ($type:ty, $prefix:literal) => {
        impl Display for $type {
            fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

impl_hir_id_display!(BlockId, "bb");
impl_hir_id_display!(FunctionId, "fn");
impl_hir_id_display!(StructId, "struct");
impl_hir_id_display!(FieldId, "field");
impl_hir_id_display!(LocalId, "l");
impl_hir_id_display!(RegionId, "r");
impl_hir_id_display!(HirNodeId, "n");
impl_hir_id_display!(HirValueId, "v");

impl Display for HirBinOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            HirBinOp::Add => write!(f, "+"),
            HirBinOp::Sub => write!(f, "-"),
            HirBinOp::Mul => write!(f, "*"),
            HirBinOp::Div => write!(f, "/"),
            HirBinOp::Mod => write!(f, "%"),
            HirBinOp::Eq => write!(f, "=="),
            HirBinOp::Ne => write!(f, "!="),
            HirBinOp::Lt => write!(f, "<"),
            HirBinOp::Le => write!(f, "<="),
            HirBinOp::Gt => write!(f, ">"),
            HirBinOp::Ge => write!(f, ">="),
            HirBinOp::And => write!(f, "&&"),
            HirBinOp::Or => write!(f, "||"),
            HirBinOp::IntDiv => write!(f, "//"),
            HirBinOp::Exponent => write!(f, "^"),
        }
    }
}

impl Display for HirUnaryOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            HirUnaryOp::Neg => write!(f, "-"),
            HirUnaryOp::Not => write!(f, "!"),
        }
    }
}

impl Display for HirNumericOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.source_name())
    }
}

#[cfg(test)]
impl Display for FallibleCarrierVariant {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            FallibleCarrierVariant::Success => write!(f, "Ok"),
            FallibleCarrierVariant::Error => write!(f, "Err"),
        }
    }
}

//! Choice constructor expression parsing.
//!
//! WHAT: parses `Choice::Variant` (unit) and `Choice::Variant(...)` (payload) expressions.
//! WHY: choice construction has distinct rules from function calls and struct constructors;
//!      unit variants use value syntax while payload variants use constructor-call syntax.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, expectations_from_constructor_fields,
    resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::constructor_views::ConstructorField;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, Expression, ExpressionKind,
};
use crate::compiler_frontend::ast::expressions::generic_nominal_inference::{
    GenericNominalConstructorInput, GenericNominalTemplate, infer_generic_nominal_constructor,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, InvalidChoiceVariantReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;

/// Parse a `Choice::Variant` or `Choice::Variant(...)` construct expression.
///
/// WHAT: resolves the variant name and validates unit-vs-payload syntax rules.
/// - Unit variants: `Choice::Variant` (no parentheses).
/// - Payload variants: `Choice::Variant(...)` with positional/named arguments.
///
/// WHY: the caller has already verified the base symbol is a choice declaration
/// and that `::` follows it.
pub(super) fn parse_choice_construct(
    token_stream: &mut FileTokens,
    choice_declaration: &Declaration,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let type_id = choice_declaration.value.type_id;

    // The declaration must carry a nominal path so we can look up variant definitions
    // and produce diagnostic type information.
    let Some(nominal_path) = type_interner.environment().nominal_path(type_id) else {
        return Err(CompilerError::compiler_error(
            "Choice construct parser was called with a declaration that has no nominal path.",
        )
        .into());
    };
    let nominal_path = nominal_path.to_owned();

    // Try the canonical type environment first; fall back to header-stage shell
    // declarations when the environment has not been populated yet.
    let variant_definitions: Vec<ChoiceVariantDefinition> = type_interner
        .environment()
        .variants_for(type_id)
        .filter(|defs| !defs.is_empty())
        .map(|defs| defs.to_vec())
        .or_else(|| {
            context
                .choice_variant_shells_by_path
                .as_ref()
                .and_then(|shells| shells.get(&nominal_path))
                .map(|shells| choice_variant_shells_to_definitions(shells))
        })
        .unwrap_or_default();

    let choice_name_str = nominal_path
        .name_str(string_table)
        .unwrap_or("<choice>")
        .to_owned();

    token_stream.advance();
    if token_stream.current_token_kind() != &TokenKind::DoubleColon {
        return Err(CompilerError::compiler_error(format!(
            "Choice construct parser expected '::' after choice name '{}'.",
            choice_name_str
        ))
        .into());
    }

    token_stream.advance();
    token_stream.skip_newlines();

    let variant_location = token_stream.current_location();
    let variant_name = match token_stream.current_token_kind() {
        TokenKind::Symbol(name) => *name,

        TokenKind::Must | TokenKind::TraitThis => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                token_stream.current_token_kind(),
                token_stream.current_location(),
                "Expression Parsing",
                "choice variant expression parsing",
            )?;

            return Err(
                reserved_trait_keyword_error(keyword, token_stream.current_location()).into(),
            );
        }

        found => {
            return Err(CompilerDiagnostic::unexpected_token(
                found.clone(),
                token_stream.current_location(),
            )
            .into());
        }
    };

    let Some(variant_index) = variant_definitions
        .iter()
        .position(|variant| variant.name == variant_name)
    else {
        let available_variant_ids: Vec<_> = variant_definitions.iter().map(|v| v.name).collect();

        return Err(CompilerDiagnostic::invalid_choice_variant(
            InvalidChoiceVariantReason::UnknownVariant,
            choice_declaration.id.name(),
            Some(variant_name),
            available_variant_ids,
            variant_location,
        )
        .into());
    };

    let variant = &variant_definitions[variant_index];
    let has_parens = token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis);
    let mut parsed_payload_arguments = None;
    let mut constructor_location = variant_location.clone();

    // Pre-parse call arguments for record variants so generic inference can
    // inspect the raw argument expressions before type instantiation. Field
    // expectations are derived from the payload shell so `cast` receives a
    // concrete target for non-generic fields and `TargetIsGenericParameter` for
    // generic parameter fields.
    if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload
        && has_parens
    {
        token_stream.advance(); // past variant name to '('
        constructor_location = token_stream.current_location();

        let payload_field_views = ConstructorField::from_choice_payload_fields(fields);
        let field_expectations = expectations_from_constructor_fields(&payload_field_views);
        let variant_name_str = string_table.resolve(variant_name).to_owned();
        let callee_name = string_table.intern(&format!("{choice_name_str}::{variant_name_str}"));
        parsed_payload_arguments = Some(parse_call_arguments_typed_with_expectations(
            token_stream,
            context,
            type_interner,
            string_table,
            &field_expectations,
            CallArgumentSyntax::Supported {
                callee_name: Some(callee_name),
            },
        )?);
    }

    let generic_declaration_metadata = context
        .generic_declarations_by_path
        .as_ref()
        .and_then(|declarations| declarations.get(&nominal_path))
        .filter(|metadata| !metadata.parameters.is_empty());

    // ---------------------------
    //  Resolve generic parameters
    // ---------------------------
    let (instantiated_variant_defs, choice_type_id, generic_instance_key) =
        if let Some(metadata) = generic_declaration_metadata {
            let constructor_fields = match &variant.payload {
                ChoiceVariantPayloadDefinition::Record { fields } => {
                    Some(ConstructorField::from_choice_payload_fields(fields))
                }
                ChoiceVariantPayloadDefinition::Unit => None,
            };
            let inference = infer_generic_nominal_constructor(
                GenericNominalConstructorInput {
                    nominal_path: &nominal_path,
                    display_name: &choice_name_str,
                    metadata,
                    template: GenericNominalTemplate::ChoiceVariants(&variant_definitions),
                    constructor_fields: constructor_fields.as_deref(),
                    raw_args: parsed_payload_arguments.as_deref(),
                    location: constructor_location.clone(),
                },
                context,
                type_interner,
                string_table,
            )?;

            let instantiated_variant_defs: Vec<ChoiceVariantDefinition> =
                if let Some(instance_type_id) = inference.instance_type_id {
                    let type_env = type_interner.environment();
                    type_env
                        .variants_for(instance_type_id)
                        .map(|defs| defs.to_vec())
                        .unwrap_or_else(|| variant_definitions.clone())
                } else {
                    variant_definitions.clone()
                };

            (
                instantiated_variant_defs,
                inference.instance_type_id.unwrap_or(type_id),
                inference.instance_key,
            )
        } else {
            (variant_definitions.clone(), type_id, None)
        };

    let selected_variant = &instantiated_variant_defs[variant_index];

    // ----------------------------------------
    //  Build unit or record choice expression
    // ----------------------------------------
    match &selected_variant.payload {
        ChoiceVariantPayloadDefinition::Unit => {
            token_stream.advance();

            if has_parens {
                token_stream.advance(); // past '('

                if token_stream.current_token_kind() == &TokenKind::CloseParenthesis {
                    return Err(CompilerDiagnostic::invalid_choice_variant(
                        InvalidChoiceVariantReason::UnitVariantWithParentheses,
                        choice_declaration.id.name(),
                        Some(variant_name),
                        vec![],
                        token_stream.current_location(),
                    )
                    .into());
                }

                return Err(CompilerDiagnostic::invalid_choice_variant(
                    InvalidChoiceVariantReason::UnitVariantAsConstructor,
                    choice_declaration.id.name(),
                    Some(variant_name),
                    vec![],
                    variant_location,
                )
                .into());
            }

            let diagnostic_type = DataType::Choices {
                nominal_path: nominal_path.clone(),
                type_id: choice_type_id,
                generic_instance_key,
            };

            let choice_expr = Expression::choice_construct(ChoiceConstructInput {
                nominal_path: nominal_path.clone(),
                tag: variant_index,
                fields: vec![],
                diagnostic_type,
                type_id: choice_type_id,
                location: variant_location,
                value_mode: ValueMode::ImmutableOwned,
            });
            Ok(choice_expr)
        }

        ChoiceVariantPayloadDefinition::Record { fields } => {
            if !has_parens {
                token_stream.advance();
                return Err(CompilerDiagnostic::invalid_choice_variant(
                    InvalidChoiceVariantReason::PayloadVariantMissingArguments,
                    choice_declaration.id.name(),
                    Some(variant_name),
                    vec![],
                    token_stream.current_location(),
                )
                .into());
            }

            // This should always be Some because record variants with parentheses
            // are pre-parsed above. A missing value indicates an internal invariant.
            let Some(raw_args) = parsed_payload_arguments.as_ref() else {
                return Err(CompilerError::compiler_error(
                    "Payload choice constructor reached validation without parsed arguments.",
                )
                .into());
            };

            let constructor_fields = ConstructorField::from_choice_payload_fields(fields);
            let expectations = expectations_from_constructor_fields(&constructor_fields);
            let type_check_context = type_interner.type_check_context();
            let resolved_args = resolve_call_arguments(
                CallDiagnosticContext::choice_constructor(&format!(
                    "{}::{}",
                    choice_name_str,
                    string_table.resolve(variant_name)
                )),
                raw_args,
                &expectations,
                constructor_location.clone(),
                CallArgumentResolutionContext {
                    string_table,
                    type_environment: type_check_context.type_environment,
                    compatibility_cache: type_check_context.compatibility_cache,
                },
            )?;

            let enforce_constant_context = context.kind.allows_const_record_coercion();
            let mut choice_fields = Vec::with_capacity(fields.len());

            for (field, arg) in fields.iter().zip(resolved_args.iter()) {
                let mut value = arg.value.clone();

                if enforce_constant_context {
                    // Header-stage choice shells may carry placeholder references until AST
                    // environment construction resolves constants in graph order.
                    let is_placeholder_reference =
                        if let ExpressionKind::Reference(path) = &value.kind {
                            path.name().is_some_and(|name| {
                                context.get_reference(&name).is_some_and(|declaration| {
                                    declaration
                                        .as_declaration()
                                        .is_unresolved_constant_placeholder()
                                })
                            })
                        } else {
                            false
                        };

                    let value_is_compile_time_constant = if is_placeholder_reference {
                        true
                    } else {
                        value
                            .const_value_kind_with_template_classifier(&mut |template| {
                                classify_template_from_effective_tir(
                                    template,
                                    &context.template_ir_store,
                                )
                            })?
                            .is_compile_time_value()
                    };

                    if !value_is_compile_time_constant {
                        let field_name = field
                            .name
                            .name()
                            .unwrap_or_else(|| string_table.intern("<field>"));
                        return Err(CompilerDiagnostic::compile_time_evaluation_error(
                            CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext,
                            Some(field_name),
                            value.location,
                        )
                        .into());
                    }

                    // Const records are data-only exports, so mutable ownership is removed to
                    // keep constant semantics explicit in later stages.
                    value.value_mode = ValueMode::ImmutableOwned;
                }

                choice_fields.push(Declaration {
                    id: field.name.clone(),
                    value,
                    config_qualifier: None,
                });
            }

            let value_mode = if enforce_constant_context {
                ValueMode::ImmutableOwned
            } else {
                // Non-const choice construction always produces a mutable owned value
                // because there is no caller-supplied ownership mode for choices.
                ValueMode::MutableOwned
            };

            let diagnostic_type = DataType::Choices {
                nominal_path: nominal_path.clone(),
                type_id: choice_type_id,
                generic_instance_key,
            };

            let choice_expr = Expression::choice_construct(ChoiceConstructInput {
                nominal_path: nominal_path.clone(),
                tag: variant_index,
                fields: choice_fields,
                diagnostic_type,
                type_id: choice_type_id,
                location: variant_location,
                value_mode,
            });
            Ok(choice_expr)
        }
    }
}

/// Convert AST-owned choice variant shells to `ChoiceVariantDefinition` for early constructor
/// validation.
///
/// WHAT: bridges unresolved `ChoiceVariant` shells (carrying `Declaration` payloads) to the
/// `ChoiceVariantDefinition` shape that call validation and generic inference expect.
/// WHY: choice constructor parsing must work before final canonical variants are written to
/// `TypeEnvironment`. The shell declarations already carry the semantic `TypeId`s produced by
/// AST resolution, so this fallback does not re-run diagnostic type conversion or add
/// `TypeEnvironment` placeholders.
fn choice_variant_shells_to_definitions(shells: &[ChoiceVariant]) -> Vec<ChoiceVariantDefinition> {
    shells
        .iter()
        .enumerate()
        .map(|(tag, variant)| ChoiceVariantDefinition {
            name: variant.id,
            tag,
            payload: match &variant.payload {
                ChoiceVariantPayload::Unit => ChoiceVariantPayloadDefinition::Unit,

                ChoiceVariantPayload::Record { fields } => {
                    let field_defs: Vec<FieldDefinition> = fields
                        .iter()
                        .map(|field| FieldDefinition {
                            name: field.id.clone(),
                            type_id: field.value.type_id,
                            location: field.value.location.clone(),
                        })
                        .collect();

                    ChoiceVariantPayloadDefinition::Record {
                        fields: field_defs.into_boxed_slice(),
                    }
                }
            },
            location: variant.location.clone(),
        })
        .collect()
}

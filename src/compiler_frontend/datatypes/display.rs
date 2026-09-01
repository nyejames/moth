//! Type name rendering through `StringTable`.
//!
//! WHAT: converts `TypeId` into human-readable strings for diagnostics and debug.
//! WHY: type identity is numeric; display is a separate concern that needs
//!      the shared `StringTable` to resolve interned names.
//!
//! Diagnostics should keep semantic `TypeId`s in their payloads and call these helpers only at the
//! render boundary through `DiagnosticRenderContext`.

use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::definitions::{ChoiceVariantPayloadDefinition, TypeDefinition};
use super::environment::TypeEnvironment;
use super::ids::{BuiltinTypeConstructor, TypeConstructor, TypeId};

// -----------------------------------------------------------
//  Public Display Interface
// -----------------------------------------------------------

/// Renders a `TypeId` to a human-readable string.
pub fn display_type(type_id: TypeId, env: &TypeEnvironment, table: &StringTable) -> String {
    match env.get(type_id) {
        None => "<unknown type>".to_owned(),
        Some(definition) => display_definition(definition, env, table),
    }
}

/// Formats the user-facing spelling for a fallible signature.
///
/// WHAT: joins already-rendered success slots with the final error slot.
/// WHY: internal fallible carriers and canonical function types both need to render
///      through the same `A, B, E!` surface while the compiler migrates away
///      from public first-class result values.
pub(crate) fn format_fallible_signature_parts(
    mut success_parts: Vec<String>,
    error_part: String,
) -> String {
    success_parts.push(format!("{error_part}!"));
    success_parts.join(", ")
}

// -----------------------------------------------------------
//  Internal Rendering Helpers
// -----------------------------------------------------------

fn display_definition(
    definition: &TypeDefinition,
    env: &TypeEnvironment,
    table: &StringTable,
) -> String {
    match definition {
        TypeDefinition::Builtin(builtin) => match builtin.key {
            super::ids::BuiltinTypeKey::Bool => "Bool".to_owned(),
            super::ids::BuiltinTypeKey::Int => "Int".to_owned(),
            super::ids::BuiltinTypeKey::Float => "Float".to_owned(),
            // Decimal is intentionally inactive in the Alpha surface.
            super::ids::BuiltinTypeKey::Decimal => "Decimal".to_owned(),
            super::ids::BuiltinTypeKey::String => "String".to_owned(),
            super::ids::BuiltinTypeKey::Char => "Char".to_owned(),
            super::ids::BuiltinTypeKey::Range => "Range".to_owned(),
            super::ids::BuiltinTypeKey::None => "None".to_owned(),
        },
        TypeDefinition::Struct(struct_def) => {
            let name = struct_def
                .path
                .name_str(table)
                .unwrap_or("<anonymous struct>");
            if struct_def.const_record {
                format!("const record {name}")
            } else {
                name.to_owned()
            }
        }
        TypeDefinition::Choice(choice_def) => {
            let name = choice_def.path.name_str(table).unwrap_or("<choice>");
            if choice_def.variants.is_empty() {
                format!("{name}::{{}}")
            } else {
                let variant_names: Vec<String> = choice_def
                    .variants
                    .iter()
                    .map(|variant| {
                        let base = table.resolve(variant.name).to_owned();
                        match &variant.payload {
                            ChoiceVariantPayloadDefinition::Unit => base,
                            ChoiceVariantPayloadDefinition::Record { .. } => format!("{base}(...)"),
                        }
                    })
                    .collect();
                format!("{name}::{{{}}}", variant_names.join(", "))
            }
        }
        TypeDefinition::Constructed(constructed) => display_constructed(constructed, env, table),
        TypeDefinition::Function(function) => {
            let param_types: Vec<String> = function
                .parameters
                .iter()
                .map(|p| display_type(p.type_id, env, table))
                .collect();

            let return_display = display_return_signature(
                function.returns.as_ref(),
                function.error_return,
                env,
                table,
            );

            format!("Function({} -> {})", param_types.join(", "), return_display)
        }
        TypeDefinition::External(external) => {
            format!("External({})", external.type_id.0)
        }
        TypeDefinition::GenericParameter(param) => table.resolve(param.name).to_owned(),
        TypeDefinition::GenericInstance(instance) => {
            let base_name = env
                .nominal_path_by_id(instance.base)
                .and_then(|path| path.name_str(table))
                .unwrap_or("<generic>");
            let args: Vec<String> = instance
                .arguments
                .iter()
                .map(|arg| display_type(*arg, env, table))
                .collect();
            format!("{base_name} of {}", args.join(", "))
        }

        // The marker is a compile-time member group, not a struct; diagnostics must not
        // render it as one.
        TypeDefinition::AnonymousConstRecordMarker => "anonymous const record".to_owned(),
    }
}

fn display_constructed(
    constructed: &super::definitions::ConstructedTypeDefinition,
    env: &TypeEnvironment,
    table: &StringTable,
) -> String {
    match &constructed.constructor {
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
            if let Some(element) = constructed.arguments.first() {
                match fixed_capacity {
                    Some(cap) => format!("{{{cap} {}}}", display_type(*element, env, table)),
                    None => format!("{{{}}}", display_type(*element, env, table)),
                }
            } else {
                "Collection".to_owned()
            }
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
            if let [key, value] = constructed.arguments.as_ref() {
                format!(
                    "{{{key_type} = {value_type}}}",
                    key_type = display_type(*key, env, table),
                    value_type = display_type(*value, env, table)
                )
            } else {
                "Map".to_owned()
            }
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
            if let Some(inner) = constructed.arguments.first() {
                format!("{}?", display_type(*inner, env, table))
            } else {
                "Option".to_owned()
            }
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier) => {
            if let [success, error] = constructed.arguments.as_ref() {
                display_fallible_carrier(*success, *error, env, table)
            } else {
                "FallibleCarrier".to_owned()
            }
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple) => {
            let fields: Vec<String> = constructed
                .arguments
                .iter()
                .map(|arg| display_type(*arg, env, table))
                .collect();
            format!("({})", fields.join(", "))
        }
    }
}

fn display_fallible_carrier(
    success_type: TypeId,
    error_type: TypeId,
    env: &TypeEnvironment,
    table: &StringTable,
) -> String {
    let success_types = if success_type == env.builtins().none {
        Vec::new()
    } else if let Some(tuple_fields) = env.tuple_field_ids(success_type) {
        tuple_fields.to_vec()
    } else {
        vec![success_type]
    };

    display_fallible_signature(&success_types, error_type, env, table)
}

fn display_return_signature(
    success_types: &[TypeId],
    error_type: Option<TypeId>,
    env: &TypeEnvironment,
    table: &StringTable,
) -> String {
    match error_type {
        Some(error_type) => display_fallible_signature(success_types, error_type, env, table),
        None => success_types
            .iter()
            .map(|success_type| display_type(*success_type, env, table))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn display_fallible_signature(
    success_types: &[TypeId],
    error_type: TypeId,
    env: &TypeEnvironment,
    table: &StringTable,
) -> String {
    let success_parts = success_types
        .iter()
        .map(|success_type| display_type(*success_type, env, table))
        .collect::<Vec<_>>();
    let error_part = display_type(error_type, env, table);

    format_fallible_signature_parts(success_parts, error_part)
}

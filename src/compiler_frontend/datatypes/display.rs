//! Type name rendering through `StringTableResolver`.
//!
//! WHAT: converts `TypeId` into human-readable strings for diagnostics and debug.
//! WHY: type identity is numeric; display is a separate concern that needs
//!      a string resolver to map interned names, whether from a mutable
//!      `StringTable` or a frozen `FrozenStringTable`.
//!
//! Diagnostics should keep semantic `TypeId`s in their payloads and call these helpers only at the
//! render boundary through `DiagnosticRenderContext`.
//!
//! The spelling rules live once, in `display_view_type`, which reads a `TypeDisplayView`. The live
//! `TypeEnvironment` and the frozen type-display snapshot both implement that view, so a snapshot
//! spells every type exactly as the live environment does.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    StringId, StringTable, StringTableResolver,
};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

use super::definitions::{ChoiceVariantPayloadDefinition, TypeDefinition};
use super::environment::TypeEnvironment;
use super::ids::{BuiltinTypeConstructor, BuiltinTypeKey, TypeConstructor, TypeId};

// -----------------------------------------------------------
//  Public Display Interface
// -----------------------------------------------------------

/// Renders a `TypeId` to a human-readable string.
#[allow(dead_code)] // Used by feature/test-gated HIR display and diagnostics tests.
pub fn display_type(
    type_id: TypeId,
    env: &TypeEnvironment,
    table: &StringTable,
    path_table: &PathTable,
) -> String {
    display_type_with_resolver(type_id, env, table, path_table)
}
/// Renders a `TypeId` through any string resolver (mutable or frozen table).
pub(crate) fn display_type_with_resolver(
    type_id: TypeId,
    env: &TypeEnvironment,
    table: &dyn StringTableResolver,
    path_table: &PathTable,
) -> String {
    display_view_type(env, type_id, table, path_table)
}

/// Formats the user-facing spelling for a fallible signature.
///
/// WHAT: joins already-rendered success slots with the final error slot.
/// WHY: the optional error channel follows the ordered success slots without introducing
///      a value type for the result list.
fn format_fallible_signature_parts(mut success_parts: Vec<String>, error_part: String) -> String {
    success_parts.push(format!("{error_part}!"));
    success_parts.join(", ")
}

// -----------------------------------------------------------
//  Internal Rendering Helpers
// -----------------------------------------------------------

fn path_name(
    path: PathId,
    path_table: &PathTable,
    string_table: &dyn StringTableResolver,
    fallback: &str,
) -> String {
    path_table
        .try_component(path)
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_else(|| fallback.to_owned())
}

// -----------------------------------------------------------
//  Shared Spelling View
// -----------------------------------------------------------

/// The spelling class of one type, carrying the data that class needs to render itself.
///
/// WHAT: names every spelling rule with its identity ids left generic, so the live environment
///       and the frozen type-display snapshot can each supply their own identity type.
/// WHY: both sources must spell identically, so each rule is written once, in `display_view_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeDisplayShape<Id> {
    /// No definition exists for the id. Renders as the unknown placeholder.
    Unknown,
    Bool,
    String,
    Char,
    Range,
    NoneType,
    Number(NumberScale),
    FixedScalar(FixedScalar),
    Struct {
        path: PathId,
        const_record: bool,
    },
    Choice {
        path: PathId,
    },
    Collection {
        fixed_capacity: Option<usize>,
    },
    OrderedMap,
    OptionType,
    Function {
        error_return: Option<Id>,
    },
    External(u32),
    GenericParameter {
        name: StringId,
    },
    GenericInstance {
        base_path: Option<PathId>,
    },
    AnonymousConstRecord,
}

/// One variant of a choice, reduced to the facts the spelling reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChoiceVariantShape {
    pub(crate) name: StringId,
    pub(crate) has_record_payload: bool,
}

/// Read access to one source of type definitions, keyed by that source's identity type.
///
/// WHAT: implemented by the live `TypeEnvironment` and by the frozen type-display snapshot.
/// WHY: `display_view_type` spells both through this trait, so the two sources cannot drift apart.
pub(crate) trait TypeDisplayView {
    /// The identity this source uses for its types.
    type Id: Copy;

    /// The spelling class of `type_id`. A missing definition is `TypeDisplayShape::Unknown`.
    fn shape(&self, type_id: Self::Id) -> TypeDisplayShape<Self::Id>;

    /// Argument ids of a constructed type or generic instance, in declaration order.
    /// Every other shape has no arguments.
    fn argument_ids(&self, type_id: Self::Id) -> &[Self::Id];

    /// Parameter type ids of a function shape, in declaration order.
    fn function_parameter_ids(&self, type_id: Self::Id) -> impl Iterator<Item = Self::Id>;

    /// Success return type ids of a function shape, in declaration order.
    fn function_return_ids(&self, type_id: Self::Id) -> impl Iterator<Item = Self::Id>;

    /// Variants of a choice shape, in declaration order.
    fn choice_variants(&self, type_id: Self::Id) -> impl Iterator<Item = ChoiceVariantShape>;

    /// True only for generic parameters, which the render layer reports as unresolved names.
    fn is_generic_parameter(&self, type_id: Self::Id) -> bool {
        matches!(
            self.shape(type_id),
            TypeDisplayShape::GenericParameter { .. }
        )
    }
}

impl TypeDisplayView for TypeEnvironment {
    type Id = TypeId;

    fn shape(&self, type_id: TypeId) -> TypeDisplayShape<TypeId> {
        let Some(definition) = self.get(type_id) else {
            return TypeDisplayShape::Unknown;
        };

        match definition {
            TypeDefinition::Builtin(builtin) => match builtin.key {
                BuiltinTypeKey::Bool => TypeDisplayShape::Bool,
                BuiltinTypeKey::String => TypeDisplayShape::String,
                BuiltinTypeKey::Char => TypeDisplayShape::Char,
                BuiltinTypeKey::Range => TypeDisplayShape::Range,
                BuiltinTypeKey::None => TypeDisplayShape::NoneType,
                BuiltinTypeKey::Number(scale) => TypeDisplayShape::Number(scale),
                BuiltinTypeKey::FixedScalar(scalar) => TypeDisplayShape::FixedScalar(scalar),
            },
            TypeDefinition::Struct(struct_definition) => TypeDisplayShape::Struct {
                path: struct_definition.path,
                const_record: struct_definition.const_record,
            },
            TypeDefinition::Choice(choice_definition) => TypeDisplayShape::Choice {
                path: choice_definition.path,
            },
            TypeDefinition::Constructed(constructed) => match &constructed.constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
                    TypeDisplayShape::Collection {
                        fixed_capacity: *fixed_capacity,
                    }
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
                    TypeDisplayShape::OrderedMap
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
                    TypeDisplayShape::OptionType
                }
            },
            TypeDefinition::Function(function) => TypeDisplayShape::Function {
                error_return: function.error_return,
            },
            TypeDefinition::External(external) => TypeDisplayShape::External(external.type_id.0),
            TypeDefinition::GenericParameter(parameter) => TypeDisplayShape::GenericParameter {
                name: parameter.name,
            },
            TypeDefinition::GenericInstance(instance) => TypeDisplayShape::GenericInstance {
                base_path: self.nominal_path_by_id(instance.base).copied(),
            },
            TypeDefinition::AnonymousConstRecordMarker => TypeDisplayShape::AnonymousConstRecord,
        }
    }

    fn argument_ids(&self, type_id: TypeId) -> &[TypeId] {
        match self.get(type_id) {
            Some(TypeDefinition::Constructed(constructed)) => &constructed.arguments,
            Some(TypeDefinition::GenericInstance(instance)) => &instance.arguments,
            _ => &[],
        }
    }

    fn function_parameter_ids(&self, type_id: TypeId) -> impl Iterator<Item = TypeId> {
        let Some(TypeDefinition::Function(function)) = self.get(type_id) else {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "function parameters are read only after the shape reports a function"
            );
        };
        function
            .parameters
            .iter()
            .map(|parameter| parameter.type_id)
    }

    fn function_return_ids(&self, type_id: TypeId) -> impl Iterator<Item = TypeId> {
        let Some(TypeDefinition::Function(function)) = self.get(type_id) else {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "function returns are read only after the shape reports a function"
            );
        };
        function.returns.iter().copied()
    }

    fn choice_variants(&self, type_id: TypeId) -> impl Iterator<Item = ChoiceVariantShape> {
        let Some(TypeDefinition::Choice(choice)) = self.get(type_id) else {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "choice variants are read only after the shape reports a choice"
            );
        };
        choice.variants.iter().map(|variant| ChoiceVariantShape {
            name: variant.name,
            has_record_payload: matches!(
                variant.payload,
                ChoiceVariantPayloadDefinition::Record { .. }
            ),
        })
    }
}

// -----------------------------------------------------------
//  Shared Spelling
// -----------------------------------------------------------

/// The spelling of the builtin `Bool`. Its shape alone names it, so a renderer that names `Bool` without a captured
/// type reads this rather than a type environment.
pub(crate) const BOOL_TYPE_SPELLING: &str = "Bool";

/// Renders one type through a view. The live environment and the frozen snapshot both call this,
/// so each spelling rule exists exactly once.
pub(crate) fn display_view_type<View: TypeDisplayView>(
    view: &View,
    type_id: View::Id,
    table: &dyn StringTableResolver,
    path_table: &PathTable,
) -> String {
    match view.shape(type_id) {
        TypeDisplayShape::Unknown => "<unknown type>".to_owned(),
        TypeDisplayShape::Bool => BOOL_TYPE_SPELLING.to_owned(),
        TypeDisplayShape::String => "String".to_owned(),
        TypeDisplayShape::Char => "Char".to_owned(),
        TypeDisplayShape::Range => "Range".to_owned(),
        TypeDisplayShape::NoneType => "None".to_owned(),
        TypeDisplayShape::Number(scale) => scale.to_string(),
        TypeDisplayShape::FixedScalar(scalar) => scalar.name().to_owned(),
        TypeDisplayShape::Struct { path, const_record } => {
            let name = path_name(path, path_table, table, "<anonymous struct>");
            if const_record {
                format!("const record {name}")
            } else {
                name
            }
        }
        TypeDisplayShape::Choice { path } => {
            let name = path_name(path, path_table, table, "<choice>");
            let variant_names: Vec<String> = view
                .choice_variants(type_id)
                .map(|variant| {
                    let base = table.resolve(variant.name).to_owned();
                    if variant.has_record_payload {
                        format!("{base}(...)")
                    } else {
                        base
                    }
                })
                .collect();
            if variant_names.is_empty() {
                format!("{name}::{{}}")
            } else {
                format!("{name}::{{{}}}", variant_names.join(", "))
            }
        }
        TypeDisplayShape::Collection { fixed_capacity } => {
            match view.argument_ids(type_id).first() {
                Some(element) => {
                    let element_display = display_view_type(view, *element, table, path_table);
                    match fixed_capacity {
                        Some(capacity) => format!("{{{capacity} {element_display}}}"),
                        None => format!("{{{element_display}}}"),
                    }
                }
                None => "Collection".to_owned(),
            }
        }
        TypeDisplayShape::OrderedMap => match view.argument_ids(type_id) {
            [key, value] => {
                let key_display = display_view_type(view, *key, table, path_table);
                let value_display = display_view_type(view, *value, table, path_table);
                format!("{{{key_display} = {value_display}}}")
            }
            _ => "Map".to_owned(),
        },
        TypeDisplayShape::OptionType => match view.argument_ids(type_id).first() {
            Some(inner) => format!("{}?", display_view_type(view, *inner, table, path_table)),
            None => "Option".to_owned(),
        },
        TypeDisplayShape::Function { error_return } => {
            let parameter_parts: Vec<String> = view
                .function_parameter_ids(type_id)
                .map(|parameter| display_view_type(view, parameter, table, path_table))
                .collect();
            let success_parts: Vec<String> = view
                .function_return_ids(type_id)
                .map(|success| display_view_type(view, success, table, path_table))
                .collect();
            let return_display = match error_return {
                Some(error) => format_fallible_signature_parts(
                    success_parts,
                    display_view_type(view, error, table, path_table),
                ),
                None => success_parts.join(", "),
            };
            format!(
                "Function({} -> {})",
                parameter_parts.join(", "),
                return_display
            )
        }
        TypeDisplayShape::External(external_id) => format!("External({external_id})"),
        TypeDisplayShape::GenericParameter { name } => table.resolve(name).to_owned(),
        TypeDisplayShape::GenericInstance { base_path } => {
            let base_name = match base_path {
                Some(path) => path_name(path, path_table, table, "<generic>"),
                None => "<generic>".to_owned(),
            };
            let argument_parts: Vec<String> = view
                .argument_ids(type_id)
                .iter()
                .map(|argument| display_view_type(view, *argument, table, path_table))
                .collect();
            format!("{base_name} of {}", argument_parts.join(", "))
        }
        TypeDisplayShape::AnonymousConstRecord => {
            // Compile-time member group, not a struct: the spelling must not suggest one.
            "anonymous const record".to_owned()
        }
    }
}

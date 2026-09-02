//! Recursive project-config schema.
//!
//! WHAT: describes the record-shaped value surface a project config accepts as one schema of
//!       densely identified nodes and fields: scalar leaves, nested record nodes, collections,
//!       optionals, required and default facts, closed string domains and a per-node
//!       unknown-field policy.
//! WHY: build-side Stage 0 validation walks folded config values against this schema instead of
//!      a flat key registry, so grouped `project` records, builder sections and entry-local
//!      config can be validated recursively as they are introduced.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use std::collections::HashSet;

/// Dense identifier of one schema node (a record surface) inside a [`ConfigSchema`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConfigSchemaNodeId(pub(crate) usize);

/// Dense identifier of one schema field inside a [`ConfigSchema`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConfigSchemaFieldId(pub(crate) usize);

/// Whether a schema node accepts field names it does not itself declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnknownFieldPolicy {
    /// Undeclared field names are accepted and retained (open record, e.g. the grouped `project`).
    Preserve,
    /// Undeclared field names are rejected (closed record, e.g. active builder sections).
    Reject,
}

/// The value shape one schema field accepts.
///
/// WHAT: scalar leaves are named directly; record-valued fields point at the schema node whose
///       fields validate the record's contents; collections and optionals wrap an inner shape
///       recursively.
/// WHY: one recursive shape vocabulary lets validation descend into nested records, collection
///       elements and option payloads without a second value model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigFieldShape {
    String,
    Int,
    Bool,
    /// A record value whose fields validate against the referenced schema node.
    Record(ConfigSchemaNodeId),
    /// A collection whose elements each validate against the inner shape. A scalar value
    /// is not promoted to a one-element collection.
    Collection(Box<ConfigFieldShape>),
    /// An optional value: `none` is accepted, present values validate against the inner shape.
    Optional(Box<ConfigFieldShape>),
}

/// One field declared by a schema node.
#[derive(Clone, Debug)]
pub struct ConfigSchemaField {
    pub name: &'static str,
    pub shape: ConfigFieldShape,
    /// Closed string domain: string values must belong to this fixed set when present.
    pub allowed_strings: Option<&'static [&'static str]>,
    /// Whether an authored record must declare this field.
    pub required: bool,
    /// Value used when an authored record omits this optional field.
    pub(crate) default: Option<PublicFoldedValue>,
}

/// One schema node: a record surface with its declared fields and unknown-field policy.
#[derive(Clone, Debug)]
pub struct ConfigSchemaNode {
    /// Diagnostic identity of the record surface, e.g. `"project record"`.
    pub name: &'static str,
    pub unknown_fields: UnknownFieldPolicy,
    fields: Vec<ConfigSchemaFieldId>,
}

impl ConfigSchemaNode {
    /// Dense field identifiers on this node, in registration order.
    pub fn field_ids(&self) -> &[ConfigSchemaFieldId] {
        &self.fields
    }
}

/// A complete recursive schema: deterministic vectors of nodes and fields with dense IDs.
#[derive(Clone, Debug)]
pub struct ConfigSchema {
    nodes: Vec<ConfigSchemaNode>,
    fields: Vec<ConfigSchemaField>,
    root: ConfigSchemaNodeId,
    frozen: bool,
}

/// One builder or tooling section schema registered on a builder surface.
///
/// WHAT: a named recursive schema plus whether the selected command requires the section.
/// WHY: the validator walks this vector generically; required membership is an explicit
///      fact rather than "the schema happens to contain fields".
#[derive(Clone, Debug)]
pub struct NamedConfigSectionSchema {
    pub name: &'static str,
    pub schema: ConfigSchema,
    pub required: bool,
}

/// The config schema roots one builder surface exposes.
///
/// WHAT: the compiler-owned grouped `project` record plus named project and entry section
///       schemas registered by the selected builder and tooling overlays.
/// WHY: project and entry schemas stay distinct, and generic validation must not hard-code
///      a builder section name.
#[derive(Clone, Debug)]
pub struct ConfigSchemas {
    project: ConfigSchema,
    project_sections: Vec<NamedConfigSectionSchema>,
    entry_sections: Vec<NamedConfigSectionSchema>,
}

impl ConfigSchemas {
    pub fn new(project: ConfigSchema) -> Self {
        Self {
            project,
            project_sections: Vec::new(),
            entry_sections: Vec::new(),
        }
    }

    pub fn project(&self) -> &ConfigSchema {
        &self.project
    }

    pub fn project_sections(&self) -> &[NamedConfigSectionSchema] {
        &self.project_sections
    }

    pub fn entry_sections(&self) -> &[NamedConfigSectionSchema] {
        &self.entry_sections
    }

    pub fn project_section(&self, name: &str) -> Option<&NamedConfigSectionSchema> {
        self.project_sections
            .iter()
            .find(|section| section.name == name)
    }

    pub fn register_project_section(&mut self, section: NamedConfigSectionSchema) {
        self.project_sections.push(section);
    }

    pub fn register_entry_section(&mut self, section: NamedConfigSectionSchema) {
        self.entry_sections.push(section);
    }

    /// Checks that every schema is frozen and that each registry has unique non-reserved names.
    pub fn validate(&self) -> Result<(), CompilerError> {
        if !self.project.is_frozen() {
            return Err(CompilerError::compiler_error(
                "the project config schema must be frozen before use",
            ));
        }

        validate_section_registry("project", &self.project_sections)?;
        validate_section_registry("entry", &self.entry_sections)?;
        Ok(())
    }
}

fn validate_section_registry(
    registry: &str,
    sections: &[NamedConfigSectionSchema],
) -> Result<(), CompilerError> {
    let mut names = HashSet::new();
    for section in sections {
        if !section.schema.is_frozen() {
            return Err(CompilerError::compiler_error(format!(
                "{registry} config schema section '{}' must be frozen before use",
                section.name
            )));
        }

        if section.name == "project" {
            return Err(CompilerError::compiler_error(format!(
                "{registry} config section name 'project' is reserved"
            )));
        }

        if !names.insert(section.name) {
            return Err(CompilerError::compiler_error(format!(
                "{registry} config section name '{}' is registered more than once",
                section.name
            )));
        }
    }

    Ok(())
}

impl ConfigSchema {
    /// Creates a schema whose root is the first registered node.
    pub fn new(root_name: &'static str, unknown_fields: UnknownFieldPolicy) -> Self {
        let mut schema = Self {
            nodes: Vec::new(),
            fields: Vec::new(),
            root: ConfigSchemaNodeId(0),
            frozen: false,
        };
        schema.root = schema
            .add_node(root_name, unknown_fields)
            .expect("a new schema is unfrozen");
        schema
    }

    pub fn root(&self) -> ConfigSchemaNodeId {
        self.root
    }

    pub fn nodes(&self) -> &[ConfigSchemaNode] {
        &self.nodes
    }

    /// Validates construction invariants and seals the schema against further registration.
    pub fn validate_and_freeze(&mut self) -> Result<(), CompilerError> {
        self.ensure_mutable()?;
        self.validate_construction()?;
        self.frozen = true;
        Ok(())
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Registers a new record node and returns its dense identifier.
    pub fn add_node(
        &mut self,
        name: &'static str,
        unknown_fields: UnknownFieldPolicy,
    ) -> Result<ConfigSchemaNodeId, CompilerError> {
        self.ensure_mutable()?;
        let node_id = ConfigSchemaNodeId(self.nodes.len());
        self.nodes.push(ConfigSchemaNode {
            name,
            unknown_fields,
            fields: Vec::new(),
        });
        Ok(node_id)
    }

    /// Registers one field on a node and returns its dense identifier.
    pub fn register_field(
        &mut self,
        node: ConfigSchemaNodeId,
        field: ConfigSchemaField,
    ) -> Result<ConfigSchemaFieldId, CompilerError> {
        self.ensure_mutable()?;
        if node.0 >= self.nodes.len() {
            return Err(CompilerError::compiler_error(format!(
                "config schema cannot register a field on unknown node {}",
                node.0
            )));
        }

        let field_id = ConfigSchemaFieldId(self.fields.len());
        self.fields.push(field);
        self.nodes[node.0].fields.push(field_id);
        Ok(field_id)
    }

    pub fn node(&self, id: ConfigSchemaNodeId) -> &ConfigSchemaNode {
        &self.nodes[id.0]
    }

    pub fn field(&self, id: ConfigSchemaFieldId) -> &ConfigSchemaField {
        &self.fields[id.0]
    }

    fn validate_construction(&self) -> Result<(), CompilerError> {
        if self.root.0 >= self.nodes.len() {
            return Err(CompilerError::compiler_error(
                "config schema root references an unknown node",
            ));
        }

        for node in &self.nodes {
            let mut seen_names = HashSet::new();
            for field_id in &node.fields {
                if field_id.0 >= self.fields.len() {
                    return Err(CompilerError::compiler_error(format!(
                        "config schema node '{}' references an unknown field",
                        node.name
                    )));
                }

                let field = &self.fields[field_id.0];
                if !seen_names.insert(field.name) {
                    return Err(CompilerError::compiler_error(format!(
                        "config schema node '{}' declares duplicate field name '{}'",
                        node.name, field.name
                    )));
                }

                validate_schema_field(self, field)?;
            }
        }

        Ok(())
    }

    fn ensure_mutable(&self) -> Result<(), CompilerError> {
        if self.frozen {
            Err(CompilerError::new(
                "a frozen config schema cannot register more nodes or fields",
                SourceLocation::default(),
                ErrorType::Compiler,
            ))
        } else {
            Ok(())
        }
    }
}

fn validate_schema_field(
    schema: &ConfigSchema,
    field: &ConfigSchemaField,
) -> Result<(), CompilerError> {
    if field.required && field.default.is_some() {
        return Err(CompilerError::compiler_error(format!(
            "config schema field '{}' cannot be both required and have a default",
            field.name
        )));
    }

    if field.allowed_strings.is_some() && !shape_accepts_closed_strings(&field.shape) {
        return Err(CompilerError::compiler_error(format!(
            "config schema field '{}' declares a closed string domain on a non-string shape",
            field.name
        )));
    }

    validate_shape_node_references(&field.shape, schema.nodes.len(), field.name)?;

    if let Some(default) = &field.default
        && !default_matches_shape(schema, default, &field.shape, field.allowed_strings)
    {
        return Err(CompilerError::compiler_error(format!(
            "config schema field '{}' default does not match its declared shape",
            field.name
        )));
    }

    Ok(())
}

fn shape_accepts_closed_strings(shape: &ConfigFieldShape) -> bool {
    match shape {
        ConfigFieldShape::String => true,
        ConfigFieldShape::Optional(inner) => shape_accepts_closed_strings(inner),
        ConfigFieldShape::Int
        | ConfigFieldShape::Bool
        | ConfigFieldShape::Record(_)
        | ConfigFieldShape::Collection(_) => false,
    }
}

fn validate_shape_node_references(
    shape: &ConfigFieldShape,
    node_count: usize,
    field_name: &'static str,
) -> Result<(), CompilerError> {
    match shape {
        ConfigFieldShape::Record(node_id) => {
            if node_id.0 >= node_count {
                Err(CompilerError::compiler_error(format!(
                    "config schema field '{}' references an unknown record node",
                    field_name
                )))
            } else {
                Ok(())
            }
        }

        ConfigFieldShape::Collection(inner) | ConfigFieldShape::Optional(inner) => {
            validate_shape_node_references(inner, node_count, field_name)
        }

        ConfigFieldShape::String | ConfigFieldShape::Int | ConfigFieldShape::Bool => Ok(()),
    }
}

fn default_matches_shape(
    schema: &ConfigSchema,
    value: &PublicFoldedValue,
    shape: &ConfigFieldShape,
    allowed_strings: Option<&'static [&'static str]>,
) -> bool {
    match shape {
        ConfigFieldShape::String => match value {
            PublicFoldedValue::String(OwnedFoldedString::Text(text)) => {
                string_in_closed_domain(text, allowed_strings)
            }
            _ => false,
        },

        ConfigFieldShape::Int => matches!(value, PublicFoldedValue::Int(_)),
        ConfigFieldShape::Bool => matches!(value, PublicFoldedValue::Bool(_)),
        ConfigFieldShape::Record(node_id) => default_matches_record(schema, *node_id, value),

        ConfigFieldShape::Collection(element) => match value {
            PublicFoldedValue::Collection(items) => items
                .iter()
                .all(|item| default_matches_shape(schema, item, element, allowed_strings)),
            _ => false,
        },

        ConfigFieldShape::Optional(inner) => match value {
            PublicFoldedValue::OptionNone => true,
            PublicFoldedValue::OptionSome(inner_value) => {
                default_matches_shape(schema, inner_value, inner, allowed_strings)
            }
            other => default_matches_shape(schema, other, inner, allowed_strings),
        },
    }
}

fn default_matches_record(
    schema: &ConfigSchema,
    node_id: ConfigSchemaNodeId,
    value: &PublicFoldedValue,
) -> bool {
    let PublicFoldedValue::Record(fields) = value else {
        return false;
    };
    let Some(node) = schema.nodes().get(node_id.0) else {
        return false;
    };

    let mut seen_names = HashSet::new();
    for field in fields {
        if !seen_names.insert(field.name.as_str()) {
            return false;
        }

        match node
            .field_ids()
            .iter()
            .map(|field_id| schema.field(*field_id))
            .find(|declared| declared.name == field.name)
        {
            Some(declared) => {
                if !default_matches_shape(
                    schema,
                    &field.value,
                    &declared.shape,
                    declared.allowed_strings,
                ) {
                    return false;
                }
            }
            None => {
                if node.unknown_fields == UnknownFieldPolicy::Reject {
                    return false;
                }
            }
        }
    }

    node.field_ids().iter().all(|field_id| {
        let declared = schema.field(*field_id);
        !declared.required || fields.iter().any(|field| field.name == declared.name)
    })
}

fn string_in_closed_domain(text: &str, allowed_strings: Option<&'static [&'static str]>) -> bool {
    match allowed_strings {
        Some(allowed) => allowed.contains(&text),
        None => true,
    }
}

impl ConfigSchemaField {
    pub fn string(name: &'static str) -> Self {
        Self::leaf(name, ConfigFieldShape::String)
    }

    pub fn int(name: &'static str) -> Self {
        Self::leaf(name, ConfigFieldShape::Int)
    }

    pub fn bool(name: &'static str) -> Self {
        Self::leaf(name, ConfigFieldShape::Bool)
    }

    /// A string field whose values must belong to one closed set of allowed strings.
    pub fn closed_string(name: &'static str, allowed: &'static [&'static str]) -> Self {
        let mut field = Self::leaf(name, ConfigFieldShape::String);
        field.allowed_strings = Some(allowed);
        field
    }

    /// A collection of strings. A scalar string is not promoted to a one-element collection.
    pub fn string_collection(name: &'static str) -> Self {
        Self::leaf(
            name,
            ConfigFieldShape::Collection(Box::new(ConfigFieldShape::String)),
        )
    }

    /// A collection whose elements validate against `element`.
    pub fn collection(name: &'static str, element: ConfigFieldShape) -> Self {
        Self::leaf(name, ConfigFieldShape::Collection(Box::new(element)))
    }

    /// A record-valued field whose contents validate against the referenced node.
    pub fn record(name: &'static str, node: ConfigSchemaNodeId) -> Self {
        Self::leaf(name, ConfigFieldShape::Record(node))
    }

    /// An optional field: `none` is accepted and present values validate against the inner shape.
    pub fn optional(name: &'static str, inner: ConfigFieldShape) -> Self {
        Self::leaf(name, ConfigFieldShape::Optional(Box::new(inner)))
    }

    /// Marks the field as required on its owning node.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
    /// Supplies the value used when an authored record omits this field.
    pub(crate) fn default(mut self, value: PublicFoldedValue) -> Self {
        self.default = Some(value);
        self
    }

    /// A string field whose default spelling applies when an authored record omits it.
    pub fn string_with_default(name: &'static str, default: &'static str) -> Self {
        Self::leaf(name, ConfigFieldShape::String).default(PublicFoldedValue::String(
            OwnedFoldedString::Text(default.to_owned()),
        ))
    }

    fn leaf(name: &'static str, shape: ConfigFieldShape) -> Self {
        Self {
            name,
            shape,
            allowed_strings: None,
            required: false,
            default: None,
        }
    }
}

impl ConfigFieldShape {
    /// Human-readable description of the expected value shape for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            ConfigFieldShape::String => "a string value".to_owned(),
            ConfigFieldShape::Int => "an integer value".to_owned(),
            ConfigFieldShape::Bool => "a boolean value".to_owned(),
            ConfigFieldShape::Record(_) => "a record value".to_owned(),
            ConfigFieldShape::Collection(element) => {
                format!("a collection of {}", element.plural_description())
            }
            ConfigFieldShape::Optional(inner) => format!("{} or none", inner.describe()),
        }
    }

    fn plural_description(&self) -> String {
        match self {
            ConfigFieldShape::String => "strings".to_owned(),
            ConfigFieldShape::Int => "integers".to_owned(),
            ConfigFieldShape::Bool => "booleans".to_owned(),
            ConfigFieldShape::Record(_) => "records".to_owned(),
            ConfigFieldShape::Collection(element) => {
                format!("collections of {}", element.plural_description())
            }
            ConfigFieldShape::Optional(inner) => inner.plural_description(),
        }
    }
}

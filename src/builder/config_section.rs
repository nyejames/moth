//! Builder-owned project configuration sections.
//!
//! WHAT: a builder describes the one named record its project may author in `config.moth`:
//! string and bool fields with required or defaulted rules. `finish` turns that description into
//! the config schema the compiler's generic builder-section validation walks.
//! WHY: configuration stays a compiler-owned compile-time source. The builder only declares field
//! names, value types, required status and defaults, so validation, diagnostics and the stored
//! values keep their one existing owner.

use super::Error;
use crate::build_system::output::{
    BuilderIdentity, DEV_OUTPUT_FIELD, RELEASE_OUTPUT_FIELD, register_output_root_fields,
};
use crate::builder_surface::config_schema::{
    ConfigSchema, ConfigSchemaField, NamedConfigSectionSchema, UnknownFieldPolicy,
};
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use moth_lexical::identifier::{is_identifier, is_reserved_user_name};
use std::collections::HashSet;

/// The configuration section one builder definition declares.
///
/// `BuilderDefinitionDraft::config` passes this value to the description closure. A definition has
/// at most one section, and `finish` also registers the automatic `dev_output` and `release_output`
/// fields every builder section carries.
#[derive(Debug, Clone)]
pub struct ConfigSection {
    name: String,
    strings: Vec<StringSetting>,
    bools: Vec<BoolSetting>,
}

impl ConfigSection {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            strings: Vec::new(),
            bools: Vec::new(),
        }
    }

    /// Declares a string field.
    ///
    /// The returned setting records whether the field is required and the value an omitted field
    /// reads.
    pub fn string(&mut self, name: impl Into<String>) -> &mut StringSetting {
        self.strings.push(StringSetting {
            name: name.into(),
            required: false,
            default: None,
        });
        self.strings
            .last_mut()
            .expect("the string field was just pushed")
    }

    /// Declares a bool field.
    ///
    /// The returned setting records whether the field is required and the value an omitted field
    /// reads.
    pub fn bool(&mut self, name: impl Into<String>) -> &mut BoolSetting {
        self.bools.push(BoolSetting {
            name: name.into(),
            required: false,
            default: None,
        });
        self.bools
            .last_mut()
            .expect("the bool field was just pushed")
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Validates the description and builds the section schema `finish` registers.
    ///
    /// WHAT: rejects names the compiler's config registry would reject and field combinations the
    ///       schema construction would report as internal failures, then registers the described
    ///       fields plus the shared output roots and freezes the schema.
    /// WHY: every mistake here belongs to the builder author, so `finish` must report it as an
    ///      invalid definition instead of surfacing a schema-registration failure.
    pub(crate) fn build_schema(&self) -> Result<NamedConfigSectionSchema, Error> {
        validate_section_name(&self.name)?;
        self.validate_fields()?;

        let mut schema =
            ConfigSchema::new(format!("{} section", self.name), UnknownFieldPolicy::Reject);
        let root = schema.root();

        for setting in &self.strings {
            let field = match &setting.default {
                Some(default) => {
                    ConfigSchemaField::string_with_default(setting.name.clone(), default)
                }
                None => ConfigSchemaField::string(setting.name.clone()),
            };
            let field = if setting.required {
                field.required()
            } else {
                field
            };
            schema
                .register_field(root, field)
                .expect("the builder section schema is under construction");
        }

        for setting in &self.bools {
            let field = match setting.default {
                Some(default) => ConfigSchemaField::bool(setting.name.clone())
                    .default(PublicFoldedValue::Bool(default)),
                None => ConfigSchemaField::bool(setting.name.clone()),
            };
            let field = if setting.required {
                field.required()
            } else {
                field
            };
            schema
                .register_field(root, field)
                .expect("the builder section schema is under construction");
        }

        register_output_root_fields(&mut schema, root)
            .map_err(|failure| Error::from_infrastructure_errors(failure.into()))?;
        schema
            .validate_and_freeze()
            .map_err(|failure| Error::from_infrastructure_errors(failure.into()))?;

        Ok(NamedConfigSectionSchema {
            name: self.name.clone().into(),
            schema,
            // A section is authored only when one of its fields must be, so an omitted section
            // keeps its declared defaults instead of becoming a required record.
            required: self.has_required_field(),
        })
    }

    /// Rejects field names the schema or the shared settings store cannot accept.
    fn validate_fields(&self) -> Result<(), Error> {
        let mut field_names = HashSet::new();

        for (name, required, has_default) in self
            .strings
            .iter()
            .map(|setting| {
                (
                    setting.name.as_str(),
                    setting.required,
                    setting.default.is_some(),
                )
            })
            .chain(self.bools.iter().map(|setting| {
                (
                    setting.name.as_str(),
                    setting.required,
                    setting.default.is_some(),
                )
            }))
        {
            if !is_identifier(name) {
                return Err(Error::invalid_definition(format!(
                    "Config section '{}' field '{name}' must be a Moth identifier.",
                    self.name
                )));
            }

            if is_reserved_user_name(name) {
                return Err(Error::invalid_definition(format!(
                    "Config section '{}' field '{name}' is a reserved Moth name, so a project \
                     could never author it.",
                    self.name
                )));
            }

            if matches!(name, DEV_OUTPUT_FIELD | RELEASE_OUTPUT_FIELD) {
                return Err(Error::invalid_definition(format!(
                    "Config section '{}' field '{name}' is reserved for the automatic output roots.",
                    self.name
                )));
            }

            if !field_names.insert(name) {
                return Err(Error::invalid_definition(format!(
                    "Config section '{}' declares field '{name}' more than once.",
                    self.name
                )));
            }

            if required && has_default {
                return Err(Error::invalid_definition(format!(
                    "Config section '{}' field '{name}' cannot be both required and defaulted.",
                    self.name
                )));
            }
        }

        Ok(())
    }

    fn has_required_field(&self) -> bool {
        self.strings.iter().any(|setting| setting.required)
            || self.bools.iter().any(|setting| setting.required)
    }
}

/// A declared string field of a builder configuration section.
#[derive(Debug, Clone)]
pub struct StringSetting {
    name: String,
    required: bool,
    default: Option<String>,
}

impl StringSetting {
    /// Requires the project to author this field.
    pub fn required(&mut self) -> &mut Self {
        self.required = true;
        self
    }

    /// Supplies the value the field reads when the project omits it.
    pub fn default(&mut self, value: impl Into<String>) -> &mut Self {
        self.default = Some(value.into());
        self
    }
}

/// A declared bool field of a builder configuration section.
#[derive(Debug, Clone)]
pub struct BoolSetting {
    name: String,
    required: bool,
    default: Option<bool>,
}

impl BoolSetting {
    /// Requires the project to author this field.
    pub fn required(&mut self) -> &mut Self {
        self.required = true;
        self
    }

    /// Supplies the value the field reads when the project omits it.
    pub fn default(&mut self, value: bool) -> &mut Self {
        self.default = Some(value);
        self
    }
}

/// Rejects a section name the compiler's config registries cannot accept.
///
/// Section names follow the custom-builder identity rule: a Moth identifier that is neither the
/// compiler-owned `project` record nor a first-party builder identity, including ASCII case-only
/// variants, so a custom builder cannot claim the HTML builder's section. The shared reserved-name
/// policy applies too, because a project can never declare a record with such a name.
fn validate_section_name(name: &str) -> Result<(), Error> {
    if name == "project" {
        return Err(Error::invalid_definition(
            "Config section name 'project' is reserved for the compiler-owned project record.",
        ));
    }

    BuilderIdentity::custom(name)
        .map(|_| ())
        .map_err(|reason| {
            Error::invalid_definition(format!("Config section name '{name}' {reason}."))
        })?;

    if is_reserved_user_name(name) {
        return Err(Error::invalid_definition(format!(
            "Config section name '{name}' is a reserved Moth name, so a project could never \
             author it."
        )));
    }

    Ok(())
}

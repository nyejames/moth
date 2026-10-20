//! Output-root contract that every builder section shares.
//!
//! WHAT: names the `dev_output` and `release_output` section fields, their default folders,
//!       the schema registration each builder section uses and the settings that hold
//!       validated values.
//! WHY: output roots feed output resolution for every builder. One owner keeps the names,
//!      defaults and storage from drifting between builders.

use crate::builder_surface::config_schema::{
    ConfigSchema, ConfigSchemaField, ConfigSchemaNodeId, NamedConfigSectionSchema,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// Schema field name for the development output root.
pub(crate) const DEV_OUTPUT_FIELD: &str = "dev_output";

/// Schema field name for the release output root.
pub(crate) const RELEASE_OUTPUT_FIELD: &str = "release_output";

/// Folder the development output root uses when a builder section omits it.
pub(crate) const DEFAULT_DEV_OUTPUT: &str = "dev";

/// Folder the release output root uses when a builder section omits it.
pub(crate) const DEFAULT_RELEASE_OUTPUT: &str = "release";

/// Output roots that a builder section validated, with `None` where the section set none.
///
/// WHAT: holds the `dev_output` and `release_output` values after schema defaults apply.
/// WHY: output resolution reads these values for every builder, so they live with the output
///      policy rather than in any one builder's section type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OutputRootSettings {
    pub(crate) dev_output: Option<String>,
    pub(crate) release_output: Option<String>,
}

/// Register the output-root fields on a builder section's schema root.
///
/// WHAT: adds `dev_output` and `release_output` as string fields whose defaults are the folders
///       a project uses when its section omits them.
/// WHY: every builder section carries the same fields with the same defaults, so the schema
///      registration is written once here and each builder calls it.
pub(crate) fn register_output_root_fields(
    schema: &mut ConfigSchema,
    root: ConfigSchemaNodeId,
) -> Result<(), CompilerError> {
    schema.register_field(
        root,
        ConfigSchemaField::string_with_default(DEV_OUTPUT_FIELD, DEFAULT_DEV_OUTPUT),
    )?;
    schema.register_field(
        root,
        ConfigSchemaField::string_with_default(RELEASE_OUTPUT_FIELD, DEFAULT_RELEASE_OUTPUT),
    )?;
    Ok(())
}

/// Reject a builder surface that gives the output roots to more than one section.
///
/// WHAT: finds every project section whose root declares an output-root field and fails when
///       more than one section declares them.
/// WHY: the project holds one output-root value and one authored span for both roots, so two
///      owners would replace each other's value and lose each other's provenance.
pub(crate) fn ensure_single_output_root_owner(
    sections: &[NamedConfigSectionSchema],
) -> Result<(), CompilerError> {
    let mut owners = sections.iter().filter(|section| {
        let root = section.schema.root();
        section
            .schema
            .node(root)
            .field_ids()
            .iter()
            .any(|field_id| {
                matches!(
                    section.schema.field(*field_id).name.as_ref(),
                    DEV_OUTPUT_FIELD | RELEASE_OUTPUT_FIELD
                )
            })
    });

    let Some(first) = owners.next() else {
        return Ok(());
    };
    let Some(second) = owners.next() else {
        return Ok(());
    };

    Err(CompilerError::compiler_error(format!(
        "config sections '{}' and '{}' both own the output roots",
        first.name, second.name
    )))
}

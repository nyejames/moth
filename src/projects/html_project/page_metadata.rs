//! Entry-page metadata extraction for the HTML builder.
//!
//! WHAT: reads a strict reserved subset of top-level module constants from HIR metadata.
//! WHY: page metadata should stay builder-local and deterministic without introducing new
//!      language surface area or hidden dependencies.

use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::compiler_messages::compiler_errors::compiler_error_to_diagnostic;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidPageMetadataReason};
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::hir::constants::HirConstValue;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
const PAGE_TITLE: &str = "page_title";
const PAGE_DESCRIPTION: &str = "page_description";
const PAGE_LANG: &str = "page_lang";
const PAGE_FAVICON: &str = "page_favicon";
const PAGE_BODY_STYLE: &str = "page_body_style";
const PAGE_HEAD: &str = "page_head";
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HtmlPageMetadata {
    pub title: Option<OwnedFoldedString>,
    pub description: Option<OwnedFoldedString>,
    pub lang: Option<OwnedFoldedString>,
    pub favicon: Option<OwnedFoldedString>,
    pub body_style: Option<OwnedFoldedString>,

    // This is used for <style> and to extend any html inside the <head> tag.
    pub extra_head_html: Option<OwnedFoldedString>,
}

/// One resource selected by a reserved page-metadata constant.
///
/// Metadata is a builder-owned, non-executable use. Its authored declaration location must remain
/// attached so output conflicts identify the metadata use rather than the resource-table intern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataResourceUse {
    pub(crate) origin: StableResourceOriginId,
    pub(crate) authored_location: SourceLocation,
}

/// The complete page-metadata selection for one HTML entry.
///
/// WHAT: owns the metadata values and the non-HIR resource/site-root uses selected from reserved
///       constants.
/// WHY: the HTML builder must extract reserved constants once and share that result with resource
///      planning and both document render paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HtmlPageMetadataPlan {
    pub(crate) metadata: HtmlPageMetadata,
    pub(crate) resource_uses: Vec<MetadataResourceUse>,
    pub(crate) uses_site_root: bool,
}

pub(crate) fn extract_html_page_metadata(
    hir_module: &HirModule,
    start_function: FunctionId,
    resources: &ModuleResourceTable,
    string_table: &mut StringTable,
) -> Result<HtmlPageMetadataPlan, Box<CompilerDiagnostic>> {
    let entry_scope = hir_module
        .side_table
        .function_name_path(start_function)
        .and_then(|path| path.parent());

    let entry_scope_prefix = entry_scope
        .as_ref()
        .map(|path| path.to_portable_string(string_table));

    let error_location = entry_scope
        .as_ref()
        .map(|path| SourceLocation::new(path.to_owned(), Default::default(), Default::default()))
        .unwrap_or_default();

    let mut metadata = HtmlPageMetadata::default();
    let mut resource_uses = Vec::new();
    let mut uses_site_root = false;

    for module_constant in &hir_module.module_constants {
        let Some(reserved_name) =
            reserved_metadata_name(&module_constant.name, entry_scope_prefix.as_deref())
        else {
            continue;
        };

        let key_id = string_table.intern(reserved_name);

        let value = match &module_constant.value {
            HirConstValue::String(value) => OwnedFoldedString::Text(value.to_owned()),
            HirConstValue::StructuralString { pieces } => {
                let structural_value = ConstStringValue::Pieces(pieces.clone());
                owned_folded_string_from_const_string(&structural_value, resources, string_table)
                    .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?
            }

            // Every remaining constant shape genuinely holds a non-string value.
            HirConstValue::Int(_)
            | HirConstValue::Float(_)
            | HirConstValue::Bool(_)
            | HirConstValue::Char(_)
            | HirConstValue::Collection(_)
            | HirConstValue::Record(_)
            | HirConstValue::Range(_, _)
            | HirConstValue::OptionSome(_)
            | HirConstValue::OptionNone
            | HirConstValue::Choice { .. } => {
                return Err(invalid_page_metadata_rejection(
                    key_id,
                    InvalidPageMetadataReason::NotAString,
                    &error_location,
                ));
            }
        };

        let authored_location = hir_module
            .const_facts
            .declarations
            .values()
            .find(|fact| {
                fact.declaration_path.to_portable_string(string_table) == module_constant.name
            })
            .map(|fact| fact.location.clone())
            // Hand-built unit fixtures may omit advisory facts. Production HIR always carries
            // the matching fact, and the entry scope keeps those fixtures diagnosable.
            .unwrap_or_else(|| error_location.clone());
        record_metadata_structural_uses(
            &value,
            &authored_location,
            &mut resource_uses,
            &mut uses_site_root,
        );

        let target_slot = match reserved_name {
            PAGE_TITLE => &mut metadata.title,
            PAGE_DESCRIPTION => &mut metadata.description,
            PAGE_LANG => &mut metadata.lang,
            PAGE_FAVICON => &mut metadata.favicon,
            PAGE_BODY_STYLE => &mut metadata.body_style,
            PAGE_HEAD => &mut metadata.extra_head_html,
            _ => continue,
        };

        if target_slot.is_some() {
            return Err(invalid_page_metadata_rejection(
                key_id,
                InvalidPageMetadataReason::DuplicateDeclaration,
                &error_location,
            ));
        }

        *target_slot = Some(value);
    }

    Ok(HtmlPageMetadataPlan {
        metadata,
        resource_uses,
        uses_site_root,
    })
}

fn record_metadata_structural_uses(
    value: &OwnedFoldedString,
    authored_location: &SourceLocation,
    resource_uses: &mut Vec<MetadataResourceUse>,
    uses_site_root: &mut bool,
) {
    let OwnedFoldedString::Pieces(pieces) = value else {
        return;
    };

    for piece in pieces {
        match piece {
            OwnedFoldedStringPiece::Resource(origin) => resource_uses.push(MetadataResourceUse {
                origin: origin.clone(),
                authored_location: authored_location.clone(),
            }),
            OwnedFoldedStringPiece::SiteRoot => *uses_site_root = true,
            OwnedFoldedStringPiece::Text(_) => {}
        }
    }
}

fn reserved_metadata_name<'a>(
    raw_name: &'a str,
    entry_scope_prefix: Option<&str>,
) -> Option<&'a str> {
    if is_reserved_page_key(raw_name) {
        return Some(raw_name);
    }

    let entry_scope_prefix = entry_scope_prefix?;
    let leaf_name = raw_name
        .strip_prefix(entry_scope_prefix)?
        .strip_prefix('/')?;
    is_reserved_page_key(leaf_name).then_some(leaf_name)
}

fn is_reserved_page_key(name: &str) -> bool {
    matches!(
        name,
        PAGE_TITLE | PAGE_DESCRIPTION | PAGE_LANG | PAGE_FAVICON | PAGE_BODY_STYLE | PAGE_HEAD
    )
}

/// Builds the boxed diagnostic shared by every invalid page-metadata arm.
fn invalid_page_metadata_rejection(
    key_id: StringId,
    reason: InvalidPageMetadataReason,
    location: &SourceLocation,
) -> Box<CompilerDiagnostic> {
    Box::new(CompilerDiagnostic::invalid_page_metadata(
        key_id,
        reason,
        location.clone(),
    ))
}

#[cfg(test)]
#[path = "tests/page_metadata_tests.rs"]
mod tests;

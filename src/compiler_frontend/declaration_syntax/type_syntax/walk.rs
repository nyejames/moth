//! Named-type walkers for parsed and diagnostic type surfaces.
//!
//! WHAT: visits nominal type names embedded in parsed refs and DataType diagnostics.
//! WHY: dependency discovery and validation need traversal without taking ownership of
//! type-resolution policy.

use super::*;
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::utilities::token_scan::InitializerReference;

/// One named type reference preserved from parsed type syntax.
///
/// WHAT: keeps a bare type name distinct from the complete namespace-qualified path while
/// borrowing the parsed path in place.
/// WHY: dependency ordering and alias waiting must resolve qualified names through the declaring
/// file's visibility records instead of collapsing them to a terminal component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParsedNamedTypeReference<'a> {
    Bare(StringId),
    Qualified(&'a [StringId]),
}

/// Visit every named type reference inside a `ParsedTypeRef`.
pub(crate) fn for_each_named_type_in_parsed_ref<'a>(
    parsed: &'a ParsedTypeRef,
    visitor: &mut impl FnMut(ParsedNamedTypeReference<'a>),
) {
    match parsed {
        ParsedTypeRef::Named { name, .. } => visitor(ParsedNamedTypeReference::Bare(*name)),
        ParsedTypeRef::Qualified { path, .. } => {
            visitor(ParsedNamedTypeReference::Qualified(path.as_slice()));
        }
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            for_each_named_type_in_parsed_ref(base, visitor);
            for argument in arguments {
                for_each_named_type_in_parsed_ref(argument, visitor);
            }
        }
        ParsedTypeRef::Collection { element, .. }
        | ParsedTypeRef::Optional { inner: element, .. } => {
            for_each_named_type_in_parsed_ref(element, visitor);
        }
        ParsedTypeRef::Map { key, value, .. } => {
            for_each_named_type_in_parsed_ref(key, visitor);
            for_each_named_type_in_parsed_ref(value, visitor);
        }
        ParsedTypeRef::Result { ok, err, .. } => {
            for_each_named_type_in_parsed_ref(ok, visitor);
            for_each_named_type_in_parsed_ref(err, visitor);
        }
        _ => {}
    }
}

/// Collect every bare-constant capacity reference inside a `ParsedTypeRef`.
///
/// WHAT: walks the parsed type recursively and extracts `InitializerReference` hints from
/// every `ParsedCollectionCapacity::BareConstant` node.
/// WHY: header dependency sorting needs value-namespace ordering edges for constants used in
/// fixed-collection capacity annotations. Literal capacities need no dependency edge.
pub(crate) fn collect_capacity_references_in_parsed_ref(
    parsed: &ParsedTypeRef,
    references: &mut Vec<InitializerReference>,
) {
    match parsed {
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            collect_capacity_references_in_parsed_ref(base, references);
            for argument in arguments {
                collect_capacity_references_in_parsed_ref(argument, references);
            }
        }
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            ..
        } => {
            if let Some(ParsedCollectionCapacity::BareConstant { name, location }) = fixed_capacity
            {
                references.push(InitializerReference {
                    name: *name,
                    dot_member: None,
                    location: location.clone(),
                    followed_by_call: false,
                    followed_by_choice_namespace: false,
                });
            }
            collect_capacity_references_in_parsed_ref(element, references);
        }
        ParsedTypeRef::Map { key, value, .. } => {
            collect_capacity_references_in_parsed_ref(key, references);
            collect_capacity_references_in_parsed_ref(value, references);
        }
        ParsedTypeRef::Optional { inner, .. } => {
            collect_capacity_references_in_parsed_ref(inner, references);
        }
        ParsedTypeRef::Result { ok, err, .. } => {
            collect_capacity_references_in_parsed_ref(ok, references);
            collect_capacity_references_in_parsed_ref(err, references);
        }
        _ => {}
    }
}

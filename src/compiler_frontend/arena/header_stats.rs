//! Cheap module-wide header statistics for arena capacity estimates.
//!
//! WHAT: counts declaration-shaped headers, their generic parameters, signature members, choice
//!       variants, and local declaration-ordering hints during header aggregation.
//! WHY: these counts are policy-only seeds for capacity heuristics; they never affect
//!      diagnostics, ordering, lowering, type identity, or emitted artifacts.

use crate::compiler_frontend::declaration_syntax::choice::{
    ChoiceVariantPayloadSyntax, ChoiceVariantSyntax,
};
use crate::compiler_frontend::declaration_syntax::signature_members::FunctionSignatureSyntax;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};

/// Cheap header counts gathered during module-wide header aggregation.
///
/// WHAT: a small, Copy-able snapshot of top-level declaration volume and structural detail.
///       It carries no interned string IDs, so it needs no string-table remap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeaderStats {
    pub functions: usize,
    pub constants: usize,
    pub structs: usize,
    pub choices: usize,
    pub type_aliases: usize,
    pub traits: usize,
    pub conformances: usize,
    pub trait_incompatibilities: usize,
    pub const_templates: usize,
    pub start_functions: usize,
    pub dependency_clauses: usize,
    pub generic_parameters: usize,
    pub signature_members: usize,
    pub choice_variants: usize,
    pub local_ordering_hints: usize,
}

impl HeaderStats {
    /// Compute header statistics from the module-wide header list and symbol package.
    ///
    /// WHAT: walks the already-aggregated headers once and supplements with dependency-clause
    ///       counts from the module symbol package.
    /// WHY: header parsing has already done the expensive work; this is one cheap linear scan
    ///      that avoids rediscovering top-level declarations.
    pub(crate) fn from_headers_and_symbols(
        headers: &[Header],
        module_symbols: &ModuleSymbols,
    ) -> Self {
        let mut stats = HeaderStats::default();

        for header in headers {
            stats.accumulate(header);
        }

        stats.dependency_clauses = module_symbols
            .file_dependency_clauses_by_source
            .values()
            .map(Vec::len)
            .sum();

        stats
    }

    /// Update counters for a single header.
    ///
    /// WHAT: classifies one header and its nested syntactic children into the cheap buckets used
    ///       for capacity estimates.
    fn accumulate(&mut self, header: &Header) {
        match &header.kind {
            HeaderKind::Function {
                generic_parameters,
                signature,
            } => {
                self.functions += 1;
                self.generic_parameters += generic_parameters.parameters.len();
                self.signature_members += function_signature_member_count(signature);
            }

            HeaderKind::Constant { .. } => {
                self.constants += 1;
            }

            HeaderKind::Struct {
                generic_parameters,
                fields,
            } => {
                self.structs += 1;
                self.generic_parameters += generic_parameters.parameters.len();
                self.signature_members += fields.len();
            }

            HeaderKind::Choice {
                generic_parameters,
                variants,
            } => {
                self.choices += 1;
                self.generic_parameters += generic_parameters.parameters.len();
                self.choice_variants += variants.len();
                for variant in variants {
                    self.signature_members += variant.payload_field_count();
                }
            }

            HeaderKind::TypeAlias { .. } => {
                self.type_aliases += 1;
            }

            HeaderKind::ConstTemplate { .. } => {
                self.const_templates += 1;
            }

            HeaderKind::StartFunction => {
                self.start_functions += 1;
            }

            HeaderKind::Trait { declaration } => {
                self.traits += 1;
                for requirement in &declaration.requirements {
                    self.signature_members +=
                        function_signature_member_count(&requirement.signature);
                }
            }

            HeaderKind::TraitConformance { .. } => {
                self.conformances += 1;
            }

            HeaderKind::TraitIncompatibility { .. } => {
                self.trait_incompatibilities += 1;
            }
        }

        self.local_ordering_hints += header.local_ordering_hints.len();
    }
}

fn function_signature_member_count(signature: &FunctionSignatureSyntax) -> usize {
    signature
        .parameters
        .len()
        .saturating_add(signature.returns.len())
}

/// Trait extension to count payload fields without leaking variant internals.
///
/// WHAT: hides the choice-variant payload shape behind one numeric question.
/// WHY: keeps `HeaderStats` from depending on the full variant-payload enum.
trait ChoiceVariantFieldCount {
    fn payload_field_count(&self) -> usize;
}

impl ChoiceVariantFieldCount for ChoiceVariantSyntax {
    fn payload_field_count(&self) -> usize {
        match &self.payload {
            ChoiceVariantPayloadSyntax::Unit => 0,
            ChoiceVariantPayloadSyntax::Record { fields } => fields.len(),
        }
    }
}

//! Type alias target resolution.
//!
//! WHAT: resolves parsed type-alias targets against the canonical module `TypeEnvironment`.
//! WHY: type aliases are compile-time-only type metadata; local nominal identities must already
//! exist so alias targets can name them, and alias targets must be fully resolved before function
//! signatures and struct fields are resolved.
//!
//! ## Cycle handling
//!
//! Type alias cycles (e.g. `A as B` + `B as A`) are detected by dependency sorting, because
//! `create_header` collects named-type dependency edges from alias targets just like from struct
//! fields and constant type annotations. Self-reference (`A as A`) also creates a self-loop edge.

use crate::compiler_frontend::ast::module_ast::environment::builder::{
    AstModuleEnvironmentBuilder, DeclarationPassLanes,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedTypeAlias, ResolvedTypeAnnotation, resolve_diagnostic_type_to_type_id_checked,
    resolve_parsed_type_annotation,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::type_syntax::for_each_named_type_in_parsed_ref;
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::headers::{
    VisibleNamedTypeResolution, resolve_visible_named_type_path,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashSet;
use std::rc::Rc;
use std::sync::Arc;

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    /// Collect aliases that must wait for the constant pass.
    ///
    /// WHAT: returns the alias paths whose targets fold a `#capacity` constant, plus the aliases
    /// that name one of those aliases.
    /// WHY: an alias target that depends on a constant cannot be published before that constant
    /// is folded, and no consumer may observe a provisional target. Stage 3 already sorts the
    /// alias lane by dependency, so one forward pass closes the dependency set.
    ///
    /// Membership is resolved from the target's parsed named references through the declaration
    /// file's visibility package. Bare and qualified references both compare their canonical
    /// declaration paths, so aliases with the same terminal name under different namespaces do
    /// not collide.
    pub(in crate::compiler_frontend::ast) fn aliases_waiting_for_constants(
        &mut self,
        declaration_lanes: &DeclarationPassLanes,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
    ) -> Result<FxHashSet<InternedPath>, CompilerMessages> {
        let mut waiting = FxHashSet::default();
        for &declaration_id in &declaration_lanes.aliases {
            let header = declaration_lanes
                .header(declaration_id, sorted_headers)
                .map_err(|error| self.error_messages(error, string_table))?;
            let target = self.alias_target(header, string_table)?;

            if !header.capacity_references.is_empty() {
                waiting.insert(header.tokens.src_path.to_owned());
                continue;
            }

            let visibility = self.header_visibility(header, string_table)?;
            let mut depends_on_waiting = false;
            for_each_named_type_in_parsed_ref(target, &mut |type_reference| {
                if let VisibleNamedTypeResolution::Declaration(alias_path) =
                    resolve_visible_named_type_path(type_reference, &visibility)
                    && waiting.contains(&alias_path)
                {
                    depends_on_waiting = true;
                }
            });
            if depends_on_waiting {
                waiting.insert(header.tokens.src_path.to_owned());
            }
        }

        Ok(waiting)
    }

    /// Resolve alias targets after local nominal identities exist.
    ///
    /// WHAT: publishes every alias-lane target that does not depend on a module constant.
    /// WHY: local aliases may name structs and choices registered in the identity phase. Member
    /// shells then consume those aliases. Constant-dependent aliases are published later by the
    /// constant pass at their own Stage 3 position.
    pub(in crate::compiler_frontend::ast) fn resolve_type_aliases(
        &mut self,
        declaration_lanes: &DeclarationPassLanes,
        sorted_headers: &[Header],
        waiting_for_constants: &FxHashSet<InternedPath>,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for &declaration_id in &declaration_lanes.aliases {
            let header = declaration_lanes
                .header(declaration_id, sorted_headers)
                .map_err(|error| self.error_messages(error, string_table))?;
            if waiting_for_constants.contains(&header.tokens.src_path) {
                continue;
            }

            self.resolve_one_type_alias(header, string_table)?;
        }

        Ok(())
    }

    /// Require one completed alias row for every local alias declaration.
    ///
    /// WHAT: checks the alias lane once the alias pass and the bounded prefix walk have run, and
    /// fails at the alias declaration when its target was never published.
    /// WHY: `ResolvedTypeAlias` makes a malformed present row unrepresentable, but not an
    /// omitted one. Establishing table completeness at the producer boundary, before trait
    /// resolution reads the first alias, means every consumer may read a local alias row as a
    /// fact rather than defensively re-deriving it.
    pub(in crate::compiler_frontend::ast) fn validate_resolved_alias_completeness(
        &mut self,
        declaration_lanes: &DeclarationPassLanes,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for &declaration_id in &declaration_lanes.aliases {
            let header = declaration_lanes
                .header(declaration_id, sorted_headers)
                .map_err(|error| self.error_messages(error, string_table))?;
            if self
                .resolved_type_aliases_by_path
                .contains_key(&header.tokens.src_path)
            {
                continue;
            }

            let error = CompilerError::new(
                format!(
                    "Type alias '{}' was never published with a completed target type.",
                    header.tokens.src_path.to_string(string_table),
                ),
                header.name_location.clone(),
                ErrorType::Compiler,
            );
            return Err(self.error_messages(error, string_table));
        }

        Ok(())
    }

    /// Resolve and publish one alias target through its declaration-file visibility.
    pub(in crate::compiler_frontend::ast) fn resolve_one_type_alias(
        &mut self,
        header: &Header,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let target = self.alias_target(header, string_table)?.clone();
        let visibility = self.header_visibility(header, string_table)?;

        // The use-site scope holds a handle on the alias table, so it must end before the table
        // is mutated. Publishing while it lives deep-clones every alias already published.
        let alias = {
            let scope_context = self
                .environment_header_scope(header, string_table)
                .with_file_visibility(Arc::clone(&visibility))
                .with_resolved_module_constants(Rc::clone(&self.resolved_module_constants));
            let target_location = parsed_type_location(&target);
            let resolved_target = self.resolve_alias_target(
                &target,
                &target_location,
                &visibility,
                &scope_context,
                string_table,
            )?;

            self.complete_alias(header, &target, resolved_target, string_table)?
        };

        Rc::make_mut(&mut self.resolved_type_aliases_by_path)
            .insert(header.tokens.src_path.to_owned(), alias);

        Ok(())
    }

    fn alias_target<'header>(
        &mut self,
        header: &'header Header,
        string_table: &mut StringTable,
    ) -> Result<&'header ParsedTypeRef, CompilerMessages> {
        let HeaderKind::TypeAlias { target } = &header.kind else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Type-alias declaration lane contained a different header kind.",
                ),
                string_table,
            ));
        };

        Ok(target)
    }

    fn resolve_alias_target(
        &mut self,
        target: &ParsedTypeRef,
        target_location: &SourceLocation,
        visibility: &FileVisibility,
        scope_context: &ScopeContext,
        string_table: &mut StringTable,
    ) -> Result<ResolvedTypeAnnotation, CompilerMessages> {
        let resolved_target = {
            let mut type_resolution_context = self.type_resolution_context_for(visibility, None);
            resolve_parsed_type_annotation(
                target.clone(),
                target_location,
                &mut type_resolution_context,
                string_table,
                Some(scope_context),
            )
        };

        resolved_target.map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))
    }

    fn complete_alias(
        &mut self,
        header: &Header,
        target: &ParsedTypeRef,
        resolved_target: ResolvedTypeAnnotation,
        string_table: &mut StringTable,
    ) -> Result<ResolvedTypeAlias, CompilerMessages> {
        // Reject aliases to external opaque types for Alpha.
        // WHAT: external types are opaque and cannot be aliased by user code.
        // WHY: aliases to opaque types would let user code pretend it owns a nominal type
        //     that it cannot construct or field-access, leading to confusing semantics.
        if let DataType::External { type_id } = &resolved_target.diagnostic_type {
            let type_name = self
                .context
                .external_package_registry
                .get_type_by_id(*type_id)
                .map(|def| def.name.to_string())
                .unwrap_or_else(|| "external".to_string());

            return Err(self.diagnostic_messages(
                CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::ExternalTypeAlias {
                        type_name: string_table.intern(&type_name),
                    },
                    header.tokens.src_path.name(),
                    header.name_location.clone(),
                ),
                string_table,
            ));
        }

        let target_location = parsed_type_location(target);
        let target_type_id = match resolved_target.type_id {
            Some(type_id) => type_id,
            None => resolve_diagnostic_type_to_type_id_checked(
                &resolved_target.diagnostic_type,
                &mut self.type_environment,
                &target_location,
            )
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?,
        };

        Ok(ResolvedTypeAlias {
            diagnostic_type: resolved_target.diagnostic_type,
            target_type_id,
            declaration_location: header.name_location.clone(),
        })
    }
}

fn parsed_type_location(parsed_type: &ParsedTypeRef) -> SourceLocation {
    match parsed_type {
        ParsedTypeRef::Named { location, .. }
        | ParsedTypeRef::Qualified { location, .. }
        | ParsedTypeRef::BuiltinBool { location }
        | ParsedTypeRef::BuiltinInt { location }
        | ParsedTypeRef::BuiltinFloat { location }
        | ParsedTypeRef::BuiltinString { location }
        | ParsedTypeRef::BuiltinChar { location }
        | ParsedTypeRef::BuiltinNone { location }
        | ParsedTypeRef::This { location }
        | ParsedTypeRef::Collection { location, .. }
        | ParsedTypeRef::Map { location, .. }
        | ParsedTypeRef::Optional { location, .. }
        | ParsedTypeRef::Result { location, .. }
        | ParsedTypeRef::Applied { location, .. } => location.clone(),
        ParsedTypeRef::Inferred => SourceLocation::default(),
    }
}

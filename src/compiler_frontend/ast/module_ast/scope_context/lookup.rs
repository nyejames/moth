//! Visible source and external symbol lookup for AST scope contexts.
//!
//! WHAT: provides `ScopeContext` methods that resolve names to local declarations,
//! source-visible symbols, receiver methods, and external package items.
//! WHY: expression and type parsing need a single, consistent lookup surface that
//! respects file-local visibility, dependency aliases and receiver-call boundaries.
//!
//! All lookups in this file are read-only with respect to `ScopeContext` state.
//! Mutations to local declarations and visibility live in sibling modules
//! (`builders`, `local_declarations`, `diagnostic_sinks`).

use super::*;
use crate::compiler_frontend::ast::ast_nodes::{DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::builtins::error_type::is_reserved_parameter_contract_symbol;
use crate::compiler_frontend::compiler_messages::{
    InvalidAssignmentTargetReason, InvalidCallShapeReason, InvalidThisUsageReason, NameNamespace,
    ReservedNameOwner,
};
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::value_mode::ValueMode;

/// Raw declaration lookup distinguishes ordinary values, pending numeric bindings and Route
/// parameters. Value consumers use `get_value_reference`; only Route argument roots use the
/// dedicated forwarding accessor.
pub(crate) enum ScopeDeclarationRef<'a> {
    Local {
        declaration: Rc<Declaration>,
        binding_span: Option<SourceSpan>,
    },
    Shared(&'a Declaration),
    PendingNumeric {
        binding: crate::compiler_frontend::ast::numeric_construction::NumericBindingId,
        binding_span: Option<SourceSpan>,
    },
    RouteParameter(RouteParameterReference<'a>),
}

/// A Route parameter visible only to the direct forwarding argument path.
pub(crate) struct RouteParameterReference<'a> {
    declaration: RouteParameterDeclaration<'a>,
    input_type: Option<TypeId>,
}

enum RouteParameterDeclaration<'a> {
    Local {
        declaration: Rc<Declaration>,
        binding_span: Option<SourceSpan>,
    },
    Shared(&'a Declaration),
}

impl RouteParameterReference<'_> {
    pub(crate) fn declaration(&self) -> &Declaration {
        match &self.declaration {
            RouteParameterDeclaration::Local { declaration, .. } => declaration,
            RouteParameterDeclaration::Shared(declaration) => declaration,
        }
    }

    pub(crate) fn input_type(&self) -> Option<TypeId> {
        self.input_type
    }
}

/// A declaration reference that is legal for ordinary value consumers.
///
/// WHAT: excludes Route parameters at the lookup boundary and preserves unresolved numeric
///       bindings without manufacturing a declaration.
/// WHY: Route parameters are capabilities, not ordinary values; all normal reads receive this
///      restricted type, while direct route forwarding uses `RouteParameterReference`.
pub(crate) enum ValueDeclarationRef<'a> {
    Local {
        declaration: Rc<Declaration>,
        binding_span: Option<SourceSpan>,
    },
    Shared(&'a Declaration),
    PendingNumeric {
        binding: crate::compiler_frontend::ast::numeric_construction::NumericBindingId,
        binding_span: Option<SourceSpan>,
    },
}

impl ValueDeclarationRef<'_> {
    /// Return a borrowed view of a declaration that already has a selected type.
    pub(crate) fn ready_declaration(&self) -> Option<&Declaration> {
        match self {
            ValueDeclarationRef::Local { declaration, .. } => Some(declaration.as_ref()),
            ValueDeclarationRef::Shared(declaration) => Some(declaration),
            ValueDeclarationRef::PendingNumeric { .. } => None,
        }
    }

    pub(crate) fn pending_binding(
        &self,
    ) -> Option<crate::compiler_frontend::ast::numeric_construction::NumericBindingId> {
        match self {
            ValueDeclarationRef::PendingNumeric { binding, .. } => Some(*binding),
            _ => None,
        }
    }

    pub(crate) fn binding_span(&self) -> Option<SourceSpan> {
        match self {
            ValueDeclarationRef::Local { binding_span, .. }
            | ValueDeclarationRef::PendingNumeric { binding_span, .. } => *binding_span,
            ValueDeclarationRef::Shared(_) => None,
        }
    }
}

impl ScopeDeclarationRef<'_> {
    pub(crate) fn binding_span(&self) -> Option<SourceSpan> {
        match self {
            ScopeDeclarationRef::Local { binding_span, .. }
            | ScopeDeclarationRef::PendingNumeric { binding_span, .. } => *binding_span,
            ScopeDeclarationRef::Shared(_) => None,
            ScopeDeclarationRef::RouteParameter(reference) => match &reference.declaration {
                RouteParameterDeclaration::Local { binding_span, .. } => *binding_span,
                RouteParameterDeclaration::Shared(_) => None,
            },
        }
    }
}

// Route parameter declarations are intentionally not exposed through `ready_declaration`.
// Ordinary consumers must use `ValueDeclarationRef`, which cannot represent a Route parameter.

/// Resolved struct constructor metadata for identifier-led expression dispatch.
///
/// WHAT: carries canonical nominal identity plus the original AST field declarations used for
/// constructor defaults.
/// WHY: constructor routing uses canonical nominal identity and each field's folded content handle.
pub(crate) struct SourceStructConstructor<'a> {
    pub(crate) struct_path: PathId,
    pub(crate) fields: &'a [Declaration],
    pub(crate) struct_value_mode: &'a ValueMode,
    pub(crate) type_id: TypeId,
}

impl ScopeContext {
    // --------------------------
    //  Symbol lookup
    // --------------------------

    pub(crate) fn get_reference<'a>(&'a self, name: &StringId) -> Option<ScopeDeclarationRef<'a>> {
        // 1. Body-local declarations shadow every source-visible declaration.
        if let Some((slot, binding_span)) = self.arena.borrow().lookup(self.current_frame_id, name)
        {
            match slot {
                LocalDeclarationSlot::Ready(declaration) => {
                    let reference = match self.route_input_type(&declaration) {
                        Some(input_type) => {
                            ScopeDeclarationRef::RouteParameter(RouteParameterReference {
                                declaration: RouteParameterDeclaration::Local {
                                    declaration,
                                    binding_span,
                                },
                                input_type,
                            })
                        }
                        None => ScopeDeclarationRef::Local {
                            declaration,
                            binding_span,
                        },
                    };
                    return Some(reference);
                }
                LocalDeclarationSlot::Pending { binding, domain } => {
                    // A pending binding from another independent body root is not visible here:
                    // its dense ID indexes that body's construction tables. The name keeps
                    // resolving through the ordinary module-visible path below, exactly as if the
                    // foreign body-local declaration did not shadow it.
                    if domain == self.numeric_domain_id() {
                        return Some(ScopeDeclarationRef::PendingNumeric {
                            binding,
                            binding_span,
                        });
                    }
                }
            }
        }

        // 2. Source-visible names → canonical declaration path.
        // Includes same-file declarations and imported source symbols (aliased or not).
        // When file_visibility is populated (production contexts), this is the
        // *only* path for cross-file name lookup. The fallback below is only for test
        // contexts that do not set file_visibility.
        // Skip receiver methods: they must be called via receiver syntax, and the
        // receiver method catalog handles their lookup.
        if let Some(file_visibility) = &self.file_visibility {
            if let Some(canonical_path) = file_visibility.visible_source_names.get(name)
                && let Some(declaration) = self
                    .shared
                    .top_level_declarations
                    .get_by_path(canonical_path)
                && !declaration.is_receiver_function()
            {
                return Some(match self.route_input_type(declaration) {
                    Some(input_type) => {
                        ScopeDeclarationRef::RouteParameter(RouteParameterReference {
                            declaration: RouteParameterDeclaration::Shared(declaration),
                            input_type,
                        })
                    }
                    None => ScopeDeclarationRef::Shared(declaration),
                });
            }
            // file_visibility is set but name not found — do not fall back.
            // This ensures dependency aliases hide the original name.
            return None;
        }
        self.shared
            .top_level_declarations
            .get_visible_non_receiver_by_name(*name, self.visible_declaration_ids.as_deref())
            .map(|declaration| match self.route_input_type(declaration) {
                Some(input_type) => ScopeDeclarationRef::RouteParameter(RouteParameterReference {
                    declaration: RouteParameterDeclaration::Shared(declaration),
                    input_type,
                }),
                None => ScopeDeclarationRef::Shared(declaration),
            })
    }
    /// The outer `None` means this is not a Route parameter; the inner `None` means zero input.
    fn route_input_type(&self, declaration: &Declaration) -> Option<Option<TypeId>> {
        self.enclosing_callable
            .as_deref()
            .and_then(|callable| callable.parameter_contract(declaration.id))
            .and_then(|contract| match contract {
                ParameterContract::Route { input } => Some(input),
                ParameterContract::Value | ParameterContract::Wire => None,
            })
    }

    /// Resolve a Route parameter only for the direct call-argument forwarding path.
    pub(crate) fn get_route_argument_reference(
        &self,
        name: &StringId,
    ) -> Option<RouteParameterReference<'_>> {
        match self.get_reference(name)? {
            ScopeDeclarationRef::RouteParameter(reference) => Some(reference),
            ScopeDeclarationRef::Local { .. }
            | ScopeDeclarationRef::Shared(_)
            | ScopeDeclarationRef::PendingNumeric { .. } => None,
        }
    }

    /// Resolve an ordinary value name, rejecting Route parameters at the shared lookup boundary.
    pub(crate) fn get_value_reference<'a>(
        &'a self,
        name: &StringId,
        span: Option<SourceSpan>,
    ) -> Result<Option<ValueDeclarationRef<'a>>, CompilerDiagnostic> {
        match self.get_reference(name) {
            Some(ScopeDeclarationRef::Local {
                declaration,
                binding_span,
            }) => Ok(Some(ValueDeclarationRef::Local {
                declaration,
                binding_span,
            })),
            Some(ScopeDeclarationRef::Shared(declaration)) => {
                Ok(Some(ValueDeclarationRef::Shared(declaration)))
            }
            Some(ScopeDeclarationRef::PendingNumeric {
                binding,
                binding_span,
            }) => Ok(Some(ValueDeclarationRef::PendingNumeric {
                binding,
                binding_span,
            })),
            Some(ScopeDeclarationRef::RouteParameter(_)) => {
                Err(CompilerDiagnostic::invalid_call_shape(
                    InvalidCallShapeReason::RouteParameterRead,
                    None,
                    span,
                ))
            }
            None => Ok(None),
        }
    }

    /// Reject body bindings that would hide a bound or compiler-owned name.
    ///
    /// Declaration lookup can miss compile-time names and source declarations not yet emitted.
    /// Consume their existing visibility owners without changing value lookup or assignment.
    pub(crate) fn check_body_binding_visibility(
        &self,
        name: StringId,
        span: Option<SourceSpan>,
        string_table: &StringTable,
    ) -> Option<CompilerDiagnostic> {
        if self
            .trait_environment()
            .core_trait_id_for_name(name, string_table)
            .is_some()
        {
            return Some(CompilerDiagnostic::reserved_name_collision(
                name,
                ReservedNameOwner::CoreTrait,
                span,
            ));
        }

        // `Wire` and `Route` are compiler-owned parameter-contract heads. Every binding kind that
        // shares this check (parameters, loop, match, multi-bind, catch and body-local
        // declarations) must reserve them, because the contract spelling is only meaningful in
        // parameter position and a same-named binding would make the signature ambiguous.
        if is_reserved_parameter_contract_symbol(string_table.resolve(name)) {
            return Some(CompilerDiagnostic::reserved_name_collision(
                name,
                ReservedNameOwner::ParameterContract,
                span,
            ));
        }

        if self.file_visibility.as_ref().is_some_and(|visibility| {
            visibility.visible_source_names.contains_key(&name)
                || visibility.visible_type_alias_names.contains_key(&name)
                || visibility.visible_trait_names.contains_key(&name)
                || visibility.visible_external_symbols.contains_key(&name)
                || visibility.visible_namespace_records.contains_key(&name)
        }) {
            return Some(CompilerDiagnostic::shadowed_name(name, None, span));
        }
        None
    }

    /// Resolve the reserved receiver binding and report its shared availability rules.
    pub(crate) fn resolve_this_reference(
        &self,
        this_id: StringId,
        span: SourceSpan,
    ) -> Result<ValueDeclarationRef<'_>, CompilerDiagnostic> {
        if self.is_assignment_target_unavailable(this_id) {
            return Err(CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
                Some(this_id),
                None,
                None,
                None,
                None,
                Some(span),
            ));
        }

        self.get_value_reference(&this_id, Some(span))?
            .ok_or_else(|| {
                CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::NotInReceiverMethod,
                    Some(span),
                )
            })
    }

    /// Resolve a source-visible value name for a place root, preserving namespace diagnostics.
    pub(crate) fn resolve_value_reference(
        &self,
        name: StringId,
        span: SourceSpan,
    ) -> Result<ValueDeclarationRef<'_>, CompilerDiagnostic> {
        self.get_value_reference(&name, Some(span))?.ok_or_else(|| {
            if self.is_visible_type_alias_name(name) {
                CompilerDiagnostic::namespace_misuse(
                    name,
                    NameNamespace::Value,
                    NameNamespace::Type,
                    Some(span),
                )
            } else {
                CompilerDiagnostic::unknown_value_name(name, Some(span))
            }
        })
    }

    /// Check local and source-visible declarations before publishing a new binding.
    ///
    /// Count no-shadowing queries separately while using the same visibility owner as expressions.
    /// Other bound and compiler-owned names are checked by `check_body_binding_visibility`.
    pub(crate) fn has_visible_declaration(&self, name: &StringId) -> bool {
        increment_ast_counter(AstCounter::ScopeFrameRedeclarationAncestorChecks);
        self.get_reference(name).is_some()
    }
    /// WHAT: accepts body-local constants recorded during scope growth plus top-level
    /// module constants from either seeded header contexts or completed module lookups.
    /// WHY: fixed-capacity type syntax must reject foldable runtime bindings while still
    /// allowing visible explicit constants before and after the final lookup package exists.
    pub(crate) fn is_explicit_compile_time_constant(&self, declaration: &Declaration) -> bool {
        if self
            .arena
            .borrow()
            .is_explicit_compile_time_constant(self.current_frame_id, declaration)
        {
            return true;
        }

        let Some(declaration_id) = self
            .shared
            .top_level_declarations
            .declaration_id_by_path(&declaration.id)
        else {
            return false;
        };
        if let Some(constants) = &self.shared.resolved_module_constants_override {
            return constants.contains(declaration_id);
        }
        self.shared
            .lookups
            .as_ref()
            .is_some_and(|lookups| lookups.resolved_module_constants.contains(declaration_id))
    }

    /// Whether a declaration is a top-level constant header whose folded value is still pending.
    ///
    /// WHAT: delegates to the shared top-level table, which owns the constant header role.
    /// WHY: deferred struct-shell and constant-context checks must recognise pending constants
    ///     by header role, not by declared type.
    pub(crate) fn is_unresolved_constant_placeholder(&self, declaration: &Declaration) -> bool {
        self.shared
            .top_level_declarations
            .is_unresolved_constant_placeholder(declaration)
    }

    pub(crate) fn lookup_generic_function_template(
        &self,
        function_path: &PathId,
    ) -> Option<&GenericFunctionTemplate> {
        self.shared
            .lookups
            .as_ref()?
            .generic_function_templates_by_path
            .get(function_path)
    }

    /// Return the semantic role of a declaration without inspecting display-only type spelling.
    ///
    /// WHAT: falls back to expression classification for body-local declarations that are
    ///       not in the precomputed semantic table. Template constness is classified through
    ///       the module store so composed views retain their exact root and overlay identity.
    pub(crate) fn semantic_kind_for_declaration(
        &self,
        declaration: &Declaration,
        expression_builder: &AstExpressionBuilder,
        const_values: &ConstValueStore,
    ) -> Result<DeclarationSemanticKind, TemplateError> {
        if let Some(kind) = self
            .lookups
            .as_ref()
            .and_then(|lookups| lookups.declaration_semantics.kind_for_path(&declaration.id))
        {
            return Ok(kind);
        }

        let kind = match &declaration.value {
            DeclarationValue::Shell(DeclarationShell::Function { .. }) => {
                DeclarationSemanticKind::Function
            }
            DeclarationValue::Shell(DeclarationShell::Struct) => DeclarationSemanticKind::Struct,
            DeclarationValue::Shell(DeclarationShell::Choice) => DeclarationSemanticKind::Choice,
            DeclarationValue::StructDefinition(_) => DeclarationSemanticKind::Value,
            DeclarationValue::Shell(DeclarationShell::TypeAlias) | DeclarationValue::Absent => {
                DeclarationSemanticKind::Value
            }
            DeclarationValue::PendingConstant | DeclarationValue::Default(_) => {
                DeclarationSemanticKind::Constant
            }
            DeclarationValue::Initializer(expression_id) => {
                let expression = expression_builder.view(*expression_id)?;
                let value_is_compile_time_constant = expression
                    .const_value_kind_with_template_classifier(const_values, &mut |template| {
                        classify_template_from_effective_tir(
                            template,
                            &self.template_ir_store,
                            expression_builder,
                            const_values,
                            Some(self.external_package_registry.as_ref()),
                        )
                    })?
                    .is_compile_time_value();

                if value_is_compile_time_constant {
                    DeclarationSemanticKind::Constant
                } else {
                    DeclarationSemanticKind::Value
                }
            }
        };

        Ok(kind)
    }

    /// Resolve callable metadata for a source declaration.
    /// Callable contracts belong to the immutable module signature table.
    pub(crate) fn source_callable_signature(
        &self,
        declaration: &Declaration,
    ) -> Option<&FunctionSignature> {
        self.lookups.as_ref().and_then(|lookups| {
            lookups
                .resolved_function_signatures_by_path
                .get(&declaration.id)
                .map(|resolved| &resolved.signature)
        })
    }

    /// Only local private no-slot calls may contribute inferred failure to an outer expression.
    ///
    /// Visibility permits the lane but never proves failure. A completed body's proof can close
    /// it; numeric work, private-call candidates and unparsed/recursive bodies stay conservative.
    /// Donor-private helpers and concrete instances retain the declaring module's private lane.
    pub(crate) fn source_call_has_private_failure_lane(&self, path: PathId) -> bool {
        let Some(lookups) = self.lookups.as_ref() else {
            return false;
        };
        let known_infallible = |path| {
            self.known_infallible_functions
                .as_ref()
                .is_some_and(|functions| functions.borrow().contains(&path))
        };
        if known_infallible(path) {
            return false;
        }
        if lookups.exported_callable_paths.contains(&path) {
            return false;
        }
        if let Some(contract) = lookups.imported_functions_by_local_path.get(&path) {
            return match &contract.target {
                SourceFunctionTarget::Local(local_path) => {
                    !lookups.exported_callable_paths.contains(local_path)
                        && !known_infallible(*local_path)
                }
                SourceFunctionTarget::ModulePrivate { .. } => true,
                SourceFunctionTarget::Generated { identity, .. } => {
                    matches!(
                        identity.declaration(),
                        GeneratedDeclarationIdentity::ModulePrivate(_)
                    )
                }
                SourceFunctionTarget::Imported { .. } => false,
            };
        }
        if let Some(template) = lookups.generic_function_templates_by_path.get(&path) {
            return match &template.declaration_identity {
                Some(GeneratedDeclarationIdentity::ModulePrivate(_)) => true,
                Some(GeneratedDeclarationIdentity::Public(_)) => false,
                None => template.body_tokens.is_some(),
            };
        }

        // Ordinary top-level targets have header-owned source membership. Body-local functions
        // have no header signature row and are necessarily private to their enclosing body.
        if let Some(source) = lookups
            .module_symbols
            .canonical_source_by_symbol_path
            .get(&path)
        {
            return lookups.module_symbols.module_file_paths.contains(source);
        }
        !lookups
            .resolved_function_signatures_by_path
            .contains_key(&path)
            && !lookups.module_symbols.module_file_paths.is_empty()
    }

    /// Resolve constructor metadata for a source struct declaration.
    pub(crate) fn source_struct_constructor<'a>(
        &'a self,
        declaration: &'a Declaration,
        type_environment: &TypeEnvironment,
        expression_builder: &AstExpressionBuilder,
        const_values: &ConstValueStore,
    ) -> Result<Option<SourceStructConstructor<'a>>, TemplateError> {
        if self.semantic_kind_for_declaration(declaration, expression_builder, const_values)?
            != DeclarationSemanticKind::Struct
        {
            return Ok(None);
        }

        let type_id = declaration.type_id;
        // The semantic check above confirmed this declaration is a struct, so
        // the nominal type path must be registered. A missing path is a compiler
        // invariant violation, not a user-facing diagnostic.
        let struct_path = type_environment
            .nominal_path(type_id)
            .ok_or_else(|| {
                TemplateError::from(CompilerError::compiler_error(
                    "Struct declaration did not resolve to a nominal type path.",
                ))
            })?
            .to_owned();
        let fields = self
            .resolved_struct_fields_by_path
            .as_ref()
            .and_then(|map| map.get(&struct_path))
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        Ok(Some(SourceStructConstructor {
            struct_path,
            fields,
            struct_value_mode: &declaration.value_mode,
            type_id,
        }))
    }

    /// Return whether a source declaration is a choice type.
    pub(crate) fn is_source_choice_declaration(
        &self,
        declaration: &Declaration,
        expression_builder: &AstExpressionBuilder,
        const_values: &ConstValueStore,
    ) -> Result<bool, TemplateError> {
        Ok(
            self.semantic_kind_for_declaration(declaration, expression_builder, const_values)?
                == DeclarationSemanticKind::Choice,
        )
    }

    pub(crate) fn lookup_receiver_method(
        &self,
        receiver: &ReceiverKey,
        method_name: StringId,
    ) -> Option<&ReceiverMethodEntry> {
        // Production contexts use the header-built file visibility to determine
        // which receiver methods are visible.
        if let Some(file_visibility) = &self.file_visibility {
            let paths = file_visibility.visible_receiver_methods.get(&method_name)?;
            for visible_method in paths {
                let entry = self
                    .receiver_methods
                    .by_function_path
                    .get(visible_method.target.local_path())?;
                if &entry.receiver == receiver {
                    return Some(entry);
                }
            }
            return None;
        }

        // Unit-test contexts can omit file visibility. Production calls always use
        // header-built visibility, so the fallback keeps tests deterministic without
        // reintroducing a separate receiver-method export flag.
        let entries = self
            .receiver_methods
            .by_receiver_and_name
            .get(&(receiver.to_owned(), method_name))?;

        let current_source_file = self.source_file_scope.as_ref()?;
        entries
            .iter()
            .find(|entry| &entry.source_file == current_source_file)
            .or_else(|| entries.first())
    }

    pub(crate) fn lookup_visible_receiver_method_by_name(
        &self,
        method_name: StringId,
    ) -> Option<&ReceiverMethodEntry> {
        // Production contexts use the header-built file visibility to determine
        // which receiver methods are visible.
        if let Some(file_visibility) = &self.file_visibility {
            let paths = file_visibility.visible_receiver_methods.get(&method_name)?;
            for visible_method in paths {
                if let Some(entry) = self
                    .receiver_methods
                    .by_function_path
                    .get(visible_method.target.local_path())
                {
                    return Some(entry);
                }
            }
            return None;
        }

        // Unit-test contexts can omit file visibility. Production calls always use
        // header-built visibility, so the fallback keeps tests deterministic without
        // reintroducing a separate receiver-method export flag.
        let current_source_file = self.source_file_scope.as_ref()?;
        let entries = self.receiver_methods.by_method_name.get(&method_name)?;

        entries
            .iter()
            .find(|entry| &entry.source_file == current_source_file)
            .or_else(|| entries.first())
    }

    /// Look up a visible external function by its source-level name.
    pub(crate) fn lookup_visible_external_function(
        &self,
        name: StringId,
    ) -> Option<(ExternalFunctionId, &ExternalFunctionDef)> {
        let file_visibility = self.file_visibility.as_ref()?;
        let symbol_id = *file_visibility.visible_external_symbols.get(&name)?;
        let ExternalSymbolId::Function(function_id) = symbol_id else {
            return None;
        };
        let definition = self
            .external_package_registry
            .get_function_by_id(function_id)?;

        Some((function_id, definition))
    }

    /// Look up a visible external type by its source-level name.
    pub(crate) fn lookup_visible_external_type(
        &self,
        name: StringId,
    ) -> Option<(ExternalTypeId, &ExternalTypeDef)> {
        let file_visibility = self.file_visibility.as_ref()?;
        let symbol_id = *file_visibility.visible_external_symbols.get(&name)?;
        let ExternalSymbolId::Type(type_id) = symbol_id else {
            return None;
        };
        self.external_package_registry
            .get_type_by_id(type_id)
            .map(|definition| (type_id, definition))
    }

    /// Look up a visible external constant by its source-level name.
    pub(crate) fn lookup_visible_external_constant(
        &self,
        name: StringId,
    ) -> Option<(ExternalConstantId, &ExternalConstantDef)> {
        let file_visibility = self.file_visibility.as_ref()?;
        let symbol_id = *file_visibility.visible_external_symbols.get(&name)?;
        let ExternalSymbolId::Constant(constant_id) = symbol_id else {
            return None;
        };
        self.external_package_registry
            .get_constant_by_id(constant_id)
            .map(|definition| (constant_id, definition))
    }

    /// Check whether a name is a visible type alias, regardless of whether its target
    /// has been resolved yet.
    ///
    /// WHAT: used by expression parsing to give a precise diagnostic when a type alias
    /// is mistakenly used in value position.
    pub(crate) fn is_visible_type_alias_name(&self, name: StringId) -> bool {
        self.shared
            .file_visibility
            .as_ref()
            .is_some_and(|file_visibility| {
                file_visibility.visible_type_alias_names.contains_key(&name)
            })
    }
}

//! Function-signature parsing and function-call AST helpers.
//!
//! WHAT: parses function signatures, return lists, and host/user call metadata used by AST construction.
//! WHY: function syntax has enough dedicated parsing and type-shape rules to live outside the general statement parser.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue};
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldError, ConstantFoldOutcome, FoldServices, fold_constant_content,
};
use crate::compiler_frontend::ast::const_values::binding::resolve_const_expression_content;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionConstructionFailure, ExpressionOperationView, ExpressionView,
    OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources, ExpressionTrailingPolicy,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedTypeAnnotation, TypeResolutionContext, TypeResolutionContextInputs,
    resolve_parsed_type_annotation,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticLabel, DiagnosticPayload,
    InvalidCollectionTypeReason, InvalidTemplateStructureReason, NameNamespace,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionSignatureSyntax, ReturnChannelSyntax, ReturnSlotSyntax, SignatureMemberSyntax,
};
use crate::compiler_frontend::declaration_syntax::type_syntax::ParsedParameterContractSyntax;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Signature parsing shares the AST error lane because default values nest an explicit
/// cursor over the frozen file-owned source owner.
type SignatureResult<T> = Result<T, ExpressionParseError>;

/// Whether a return slot carries success-channel or error-channel values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnChannel {
    Success,
    Error,
}

/// Signature return slot before its authored type annotation is resolved.
pub(crate) struct UnresolvedReturnSlot {
    pub(crate) type_id: Option<TypeId>,
    /// Authored syntax is needed only while the containing signature is resolved.
    pub(crate) type_syntax: Option<ParsedTypeRef>,
    pub(crate) channel: ReturnChannel,
}

/// Resolved signature return slot with canonical identity and channel.
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnSlot {
    /// Canonical `TypeId` for this return slot when the contract is complete.
    pub type_id: Option<TypeId>,
    pub channel: ReturnChannel,
}

/// Callable parameter facts retained only until its authored annotation is resolved.
pub(crate) struct UnresolvedFunctionParameter {
    pub(crate) id: PathId,
    pub(crate) type_id: TypeId,
    /// Authored syntax is needed only while the containing signature is resolved.
    pub(crate) type_syntax: Option<ParsedTypeRef>,
    pub(crate) value_mode: ValueMode,
    pub(crate) default: Option<ConstValueId>,
    pub(crate) binding_span: Option<SourceSpan>,
    pub(crate) contract_syntax: ParsedParameterContractSyntax,
}

/// Semantic parameter contract kept separate from the parameter's ordinary value type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterContract {
    Value,
    Wire,
    Route { input: Option<TypeId> },
}

impl ParameterContract {
    /// Type id that this contract contributes beyond the parameter's ordinary value type.
    ///
    /// WHAT: returns the resolved `Route` input type, or `None` for every other contract.
    /// WHY: one traversal owner keeps every semantic walk over parameter types from re-deriving
    ///      the same projection and silently skipping the Route input.
    pub fn extra_type_id(&self) -> Option<TypeId> {
        match self {
            ParameterContract::Route { input } => *input,
            ParameterContract::Value | ParameterContract::Wire => None,
        }
    }

    /// Every type id this contract binds, given the parameter's ordinary value type.
    ///
    /// WHAT: yields the ordinary value type first, then any contract-owned type.
    /// WHY: callers that collect, validate or project parameter types must see the Route input
    ///      too; this is the single traversal owner for that decision.
    pub fn semantic_type_ids(&self, value_type_id: TypeId) -> impl Iterator<Item = TypeId> {
        std::iter::once(value_type_id).chain(self.extra_type_id())
    }

    /// This contract with the trait self type replaced by a receiver's concrete type.
    ///
    /// WHAT: substitutes the self type inside a `Route` input with the same equality rule the
    ///       parameter's own type uses.
    /// WHY: a requirement's contracts must be compared and instantiated exactly like its
    ///      parameter types, so both substitutions live on one owner.
    pub fn with_trait_this_replaced(
        self,
        trait_this_type: TypeId,
        receiver_type_id: TypeId,
    ) -> ParameterContract {
        match self {
            ParameterContract::Route { input } => ParameterContract::Route {
                input: input.map(|input| {
                    if input == trait_this_type {
                        receiver_type_id
                    } else {
                        input
                    }
                }),
            },
            ParameterContract::Value | ParameterContract::Wire => self,
        }
    }
}

/// Canonical callable parameter contract. Defaults retain content, not executable roots.
#[derive(Clone, Debug)]
pub struct FunctionParameter {
    pub id: PathId,
    pub type_id: TypeId,
    pub value_mode: ValueMode,
    pub default: Option<ConstValueId>,
    pub binding_span: Option<SourceSpan>,
    pub contract: ParameterContract,
}

/// Short-lived signature shell that owns parsed annotations until type resolution finishes.
pub(crate) struct UnresolvedFunctionSignature {
    pub(crate) parameters: Vec<UnresolvedFunctionParameter>,
    pub(crate) returns: Vec<UnresolvedReturnSlot>,
}

/// Resolved callable contract containing canonical types but no authored type syntax.
#[derive(Clone, Debug, Default)]
pub struct FunctionSignature {
    pub parameters: Vec<FunctionParameter>,
    pub returns: Vec<ReturnSlot>,
}

impl FunctionParameter {
    /// Every canonical type id this parameter's contract touches.
    ///
    /// WHAT: yields the parameter's ordinary value type and its contract-owned type (the `Route`
    ///       input) in a stable order.
    /// WHY: generic-usage checks, import origin collection, exported-surface validation and the
    ///      private sidecar closure all reason about "the types in this signature". Routing every
    ///      one of them through this owner stops a new contract type from being silently ignored
    ///      by each walker separately.
    pub(crate) fn semantic_type_ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.contract.semantic_type_ids(self.type_id)
    }

    /// Body-scope binding row for this parameter.
    ///
    /// WHAT: rebuilds the `Declaration` that a function or start body frame stores for the parameter.
    /// WHY: the callable contract keeps the folded default as a `ConstValueId`; body frames use the
    ///      same `DeclarationValue::Default` / `Absent` shape that `signature_member_to_declaration`
    ///      produces, so scope lookups classify parameters as they did before the contract split.
    pub(crate) fn to_local_declaration(&self) -> Declaration {
        Declaration {
            id: self.id,
            type_id: self.type_id,
            type_syntax: None,
            value_mode: self.value_mode,
            value: match self.default {
                Some(default) => DeclarationValue::Default(default),
                None => DeclarationValue::Absent,
            },
            binding_span: self.binding_span,
            config_qualifier: None,
        }
    }
}

/// Transient consumer-local signature facts, with ordered success slots separate from the error.
pub(crate) struct MaterialisedFunctionSignature {
    pub(crate) signature: FunctionSignature,
    pub(crate) function_type_id: TypeId,
    pub(crate) success_return_type_ids: Vec<TypeId>,
    pub(crate) error_return_type_id: Option<TypeId>,
}

#[derive(Clone, Copy)]
pub(crate) enum SignatureTypeFallbackPolicy {
    StrictCapacity,
    AllowUnresolvedCapacity,
}

/// Whether a member default stays as authored expression content or folds to canonical content now.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignatureDefaultMode {
    /// Keep the authored default unfolded as `DeclarationValue::Initializer`. Used for struct
    /// shells, whose defaults can name module constants that are not published yet; the strict
    /// struct pass folds them.
    KeepAuthored,
    /// Parse and fold the default now to `DeclarationValue::Default`.
    FoldNow,
}

impl FunctionSignature {
    /// Canonical TypeIds for success-channel return slots.
    /// Only meaningful after type resolution.
    pub fn success_return_type_ids(&self) -> Vec<TypeId> {
        self.returns
            .iter()
            .filter(|slot| slot.channel == ReturnChannel::Success)
            .filter_map(|slot| slot.type_id)
            .collect()
    }
    /// Iterates over resolved success-channel types without materialising a temporary vector.
    pub(crate) fn success_return_type_ids_iter(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.returns
            .iter()
            .filter(|slot| slot.channel == ReturnChannel::Success)
            .filter_map(|slot| slot.type_id)
    }

    /// Canonical TypeId for exactly one success-channel return slot, if it is resolved.
    ///
    /// WHAT: projects one slot directly from the borrowed signature without allocating a result
    ///       list.
    /// WHY: callers that only need a scalar result shape must distinguish it from zero or multiple
    ///      success slots without treating an unresolved type as semantic information.
    pub(crate) fn single_success_return_type_id(&self) -> Option<TypeId> {
        let mut success_slots = self
            .returns
            .iter()
            .filter(|slot| slot.channel == ReturnChannel::Success);
        let slot = success_slots.next()?;
        if success_slots.next().is_some() {
            return None;
        }
        slot.type_id
    }

    /// The number of success-channel return slots, counted without allocating a type list.
    ///
    /// WHAT: counts declared slots, whether or not their types are resolved yet.
    /// WHY: an authored `!` needs only whether a value is declared, so a count avoids a Vec.
    pub(crate) fn success_return_slot_count(&self) -> usize {
        self.returns
            .iter()
            .filter(|slot| slot.channel == ReturnChannel::Success)
            .count()
    }

    /// Canonical TypeId for the error return slot, if any.
    /// Only meaningful after type resolution.
    pub fn error_return_type_id(&self) -> Option<TypeId> {
        self.returns
            .iter()
            .find(|slot| slot.channel == ReturnChannel::Error)
            .and_then(|slot| slot.type_id)
    }
}

/// Build a signature from retained syntax under the caller's type-resolution policy.
///
/// Declaration shells defer names or capacities only when their policy permits later resolution.
#[allow(
    clippy::too_many_arguments,
    reason = "signature lowering keeps the syntax, owner, scope, fallback policy and mutable state separate"
)]
pub(crate) fn function_signature_from_syntax_with_unresolved_types(
    syntax: &FunctionSignatureSyntax,
    source_owner: &AstCursor,
    expression_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    fallback_policy: SignatureTypeFallbackPolicy,
    path_fork: &mut PathInternerFork,
) -> SignatureResult<UnresolvedFunctionSignature> {
    let mut parameters = Vec::with_capacity(syntax.parameters.len());
    for parameter in &syntax.parameters {
        let declaration = signature_member_to_declaration(
            parameter,
            source_owner,
            expression_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            fallback_policy,
            SignatureDefaultMode::FoldNow,
            path_fork,
            TypeMismatchContext::Declaration,
        )?;
        parameters.push(UnresolvedFunctionParameter {
            id: declaration.id,
            type_id: declaration.type_id,
            type_syntax: declaration.type_syntax,
            value_mode: declaration.value_mode,
            default: declaration.value.default(),
            binding_span: declaration.binding_span,
            contract_syntax: parameter.contract.clone(),
        });
    }

    let mut returns = Vec::with_capacity(syntax.returns.len());
    for return_slot in &syntax.returns {
        returns.push(return_slot_from_syntax(
            return_slot,
            expression_context,
            type_interner,
            const_values,
            string_table,
            fallback_policy,
        )?);
    }

    Ok(UnresolvedFunctionSignature {
        parameters,
        returns,
    })
}

/// Build an unresolved return slot from parsed syntax.
fn return_slot_from_syntax(
    return_slot: &ReturnSlotSyntax,
    expression_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    fallback_policy: SignatureTypeFallbackPolicy,
) -> SignatureResult<UnresolvedReturnSlot> {
    let channel = match return_slot.channel {
        ReturnChannelSyntax::Success => ReturnChannel::Success,
        ReturnChannelSyntax::Error => ReturnChannel::Error,
    };

    let resolved = resolve_signature_type_annotation(
        &return_slot.value.type_annotation,
        return_slot.value.span,
        expression_context,
        type_interner,
        const_values,
        string_table,
    );

    let type_id = match resolved {
        Ok(annotation) => annotation.type_id,
        Err(ExpressionParseError::Diagnostic(diagnostic))
            if should_fallback_signature_type(&diagnostic, fallback_policy) =>
        {
            // Generic return types may be resolved later once the function's
            // declaration-site generic parameter scope is active.
            None
        }
        Err(diagnostic) => return Err(diagnostic),
    };

    Ok(UnresolvedReturnSlot {
        type_id,
        type_syntax: Some(return_slot.value.type_annotation.clone()),
        channel,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "signature lowering keeps the member, source cursor, scope, mutable interner/string/path state, fallback policy and diagnostic context as separate borrows"
)]
pub(crate) fn signature_member_to_declaration(
    member: &SignatureMemberSyntax,
    source_owner: &AstCursor,
    expression_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    fallback_policy: SignatureTypeFallbackPolicy,
    default_mode: SignatureDefaultMode,
    path_fork: &mut PathInternerFork,
    mismatch_context: TypeMismatchContext,
) -> SignatureResult<Declaration> {
    let resolved = resolve_signature_type_annotation(
        &member.type_annotation,
        member.span,
        expression_context,
        type_interner,
        const_values,
        string_table,
    )
    .map_err(|error| signature_member_error_with_span(error, member));

    // `builtin_type_ids::NONE` is the seeded `None` type, the same id as
    // `builtins().none`. It is not an unresolved sentinel. `Option::None` means
    // the annotation did not resolve; `Some(NONE)` means it resolved to `None`.
    // The fallback arm maps an unresolved spelling onto that same id, so the id
    // alone cannot separate a resolved `None` from a deferred destination.
    // Coerce only when the annotation resolved to some other type. Treating every
    // `Some` as a resolved destination would normalise a default against `None`.
    // Generic names and unresolved capacities stay on the later resolution path.
    let (type_id, destination_resolved) = match resolved {
        Ok(annotation) => {
            let type_id = annotation.type_id.unwrap_or(builtin_type_ids::NONE);
            let destination_resolved =
                annotation.type_id.is_some() && type_id != builtin_type_ids::NONE;
            (type_id, destination_resolved)
        }
        Err(ExpressionParseError::Diagnostic(diagnostic))
            if should_fallback_signature_type(&diagnostic, fallback_policy) =>
        {
            // Keep a useful semantic shape for constructor parsing while retaining the authored
            // annotation for the later pass that can resolve its generic name or capacity.
            let type_id = resolve_signature_type_fallback(
                &member.type_annotation,
                member.span,
                expression_context,
                type_interner,
                const_values,
                string_table,
                fallback_policy,
            )?
            .unwrap_or(builtin_type_ids::NONE);
            (type_id, false)
        }
        Err(diagnostic) => return Err(diagnostic),
    };

    let member_span = member.span;
    let value = if member.default_range.is_none() {
        DeclarationValue::Absent
    } else {
        let parsed = parse_signature_default_expression(
            member,
            source_owner,
            type_id,
            expression_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )
        .map_err(|error| signature_member_error_with_span(error, member))?;
        let default = if destination_resolved {
            // Defaults receive the declared boundary; the declaration retains its
            // annotation independently of the resulting coercion expression.
            normalise_resolved_signature_default(
                parsed,
                type_id,
                expression_builder,
                type_interner,
                member,
                mismatch_context,
            )?
        } else {
            parsed
        };
        if default_mode == SignatureDefaultMode::KeepAuthored {
            DeclarationValue::Initializer(default.id())
        } else {
            let default = fold_signature_default(
                default,
                expression_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
            )
            .map_err(|error| signature_member_error_with_span(error, member))?;
            DeclarationValue::Default(default)
        }
    };

    Ok(Declaration {
        id: member.id,
        type_id,
        type_syntax: Some(member.type_annotation.clone()),
        value_mode: member.value_mode,
        value,
        binding_span: member_span,
        config_qualifier: None,
    })
}

/// Coerce a default at its declared boundary; access belongs to the parameter contract.
fn normalise_resolved_signature_default(
    expression: OwnedExpr,
    declared_type_id: TypeId,
    expression_builder: &mut AstExpressionBuilder,
    type_interner: &AstTypeInterner<'_>,
    member: &SignatureMemberSyntax,
    mismatch_context: TypeMismatchContext,
) -> SignatureResult<OwnedExpr> {
    let value = coerce_expression_to_explicit_type_boundary(
        expression,
        declared_type_id,
        type_interner.environment(),
        mismatch_context,
        expression_builder,
    )
    .map_err(|error| {
        let error = match error {
            ExpressionConstructionFailure::Diagnostic(diagnostic) => diagnostic.into(),
            ExpressionConstructionFailure::Invariant(error) => error.into(),
        };
        signature_member_error_with_span(error, member)
    })?;
    Ok(value)
}

/// Complete compile-time defaults before a callable or constructor contract retains them.
fn fold_signature_default(
    expression: OwnedExpr,
    scope: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> SignatureResult<ConstValueId> {
    let folded = fold_constant_content(
        expression.id(),
        expression_builder,
        const_values,
        FoldServices::new(
            string_table,
            Some(scope),
            Some(scope.external_package_registry.as_ref()),
        ),
        type_interner.environment(),
    )
    .map_err(|error| match error {
        ConstantFoldError::Diagnostic(diagnostic) => ExpressionParseError::Diagnostic(diagnostic),
        ConstantFoldError::Infrastructure(error) => ExpressionParseError::Infrastructure(error),
    })?;
    let expression = match folded {
        ConstantFoldOutcome::Folded(root) | ConstantFoldOutcome::NotConstant(root) => root,
        ConstantFoldOutcome::Refused { reason, .. } => {
            return Err(reason.into_diagnostic().into());
        }
    };
    fn validate_default(
        expression: ExpressionView<'_>,
        scope: &ScopeContext,
        expressions: &AstExpressionBuilder,
        constants: &ConstValueStore,
    ) -> SignatureResult<()> {
        if let ExpressionOperationView::Reference(path) = expression.operation() {
            if constants.value_for_path(&path).is_none() {
                return Err(CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
                    None,
                    expression.span(),
                )
                .into());
            }
        } else {
            match expression.operation() {
                ExpressionOperationView::Template(template) => {
                    // WHY: a slot-insert helper is valid only inside the wrapper slot it fills.
                    // Defaults fold before normalization runs, so the helper rejection that
                    // normalization applies to an unfolded default must happen here.
                    let kind = classify_template_from_effective_tir(
                        template,
                        &scope.template_ir_store,
                        expressions,
                        constants,
                        Some(scope.external_package_registry.as_ref()),
                    )
                    .map_err(|error| match error {
                        TemplateError::Diagnostic(diagnostic) => {
                            ExpressionParseError::Diagnostic(diagnostic)
                        }
                        TemplateError::Infrastructure(error) => {
                            ExpressionParseError::Infrastructure(error)
                        }
                    })?;
                    if matches!(kind, TemplateConstValueKind::SlotInsertHelper) {
                        return Err(CompilerDiagnostic::invalid_template_structure(
                            InvalidTemplateStructureReason::HelperOutsideWrapperSlot,
                            template.span,
                        )
                        .into());
                    }
                }
                ExpressionOperationView::Constant(_)
                | ExpressionOperationView::Collection(_)
                | ExpressionOperationView::Struct { .. }
                | ExpressionOperationView::Choice { .. }
                | ExpressionOperationView::Range { .. }
                | ExpressionOperationView::Coerce { .. } => {}
                ExpressionOperationView::RouteConstruct { .. } => {
                    return Err(CompilerDiagnostic::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
                        None,
                        expression.span(),
                    )
                    .into());
                }
                _ => {
                    return Err(CompilerDiagnostic::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
                        None,
                        expression.span(),
                    )
                    .into());
                }
            }
        }
        let mut result = Ok(());
        expression.visit_children(|child| {
            if result.is_ok() {
                result = validate_default(child, scope, expressions, constants);
            }
        });
        result
    }
    validate_default(
        expression_builder.view(expression)?,
        scope,
        expression_builder,
        const_values,
    )?;
    resolve_const_expression_content(
        expression,
        expression_builder,
        const_values,
        type_interner.environment(),
        scope,
        string_table,
    )
    .map_err(|error| match error {
        ConstValueStoreError::Diagnostic(diagnostic) => {
            ExpressionParseError::Diagnostic(diagnostic)
        }
        ConstValueStoreError::Infrastructure(error) => ExpressionParseError::Infrastructure(error),
    })
}

/// Attach the authored member-name anchor to an AST diagnostic while its declaring source
/// identity is still available through the active scope.
///
/// Source-authored spans are retained directly; nested consumers can refine the exact range.
fn signature_member_error_with_span(
    error: ExpressionParseError,
    member: &SignatureMemberSyntax,
) -> ExpressionParseError {
    let ExpressionParseError::Diagnostic(mut diagnostic) = error else {
        return error;
    };

    let member_span = member.span;
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = member_span;
    } else if !diagnostic
        .labels
        .iter()
        .any(|label| label.span == member_span)
    {
        diagnostic
            .labels
            .push(DiagnosticLabel::secondary(member_span, None));
    }

    ExpressionParseError::Diagnostic(diagnostic)
}

/// Resolve a parsed type annotation inside a function-style signature.
///
/// WHAT: builds the AST type-resolution context from the active `ScopeContext`.
/// WHY: parameters, struct/choice member shells, and explicit return slots all share
///      the same fixed-capacity folding rules and alias visibility model.
fn resolve_signature_type_annotation(
    type_annotation: &ParsedTypeRef,
    span: Option<SourceSpan>,
    expression_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
) -> SignatureResult<ResolvedTypeAnnotation> {
    let mut type_resolution_context =
        TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
            declaration_table: &expression_context.top_level_declarations,
            visible_declaration_ids: expression_context.visible_declaration_ids.as_ref(),
            visible_external_symbols: expression_context
                .file_visibility
                .as_ref()
                .map(|fv| &fv.visible_external_symbols),
            visible_source_bindings: expression_context
                .file_visibility
                .as_ref()
                .map(|fv| &fv.visible_source_names),
            visible_type_aliases: expression_context
                .file_visibility
                .as_ref()
                .map(|fv| &fv.visible_type_alias_names),
            resolved_type_aliases: expression_context.resolved_type_aliases.as_deref(),
            generic_declarations_by_path: expression_context
                .generic_declarations_by_path
                .as_deref(),
            resolved_struct_fields_by_path: expression_context
                .resolved_struct_fields_by_path
                .as_deref(),
            type_environment: type_interner.environment_mut_for_derived_types(),
            const_values,
            visible_namespace_records: expression_context
                .file_visibility
                .as_ref()
                .map(|fv| &fv.visible_namespace_records),
            trait_environment: expression_context.trait_environment_override.as_deref(),
            trait_evidence_environment: None,
            visible_trait_names: expression_context
                .file_visibility
                .as_ref()
                .map(|fv| &fv.visible_trait_names),
        })
        .with_active_generic_type_context(expression_context.active_generic_type_context());

    Ok(resolve_parsed_type_annotation(
        type_annotation,
        span,
        &mut type_resolution_context,
        string_table,
        Some(expression_context),
    )?)
}

fn should_fallback_signature_type(
    diagnostic: &CompilerDiagnostic,
    fallback_policy: SignatureTypeFallbackPolicy,
) -> bool {
    match &diagnostic.payload {
        DiagnosticPayload::UnknownName {
            namespace: NameNamespace::Type,
            ..
        } => true,
        DiagnosticPayload::InvalidCollectionType {
            reason: InvalidCollectionTypeReason::CapacityNotConstant,
            ..
        } => matches!(
            fallback_policy,
            SignatureTypeFallbackPolicy::AllowUnresolvedCapacity
        ),
        _ => false,
    }
}

/// Resolve any useful partial semantic shape after the authored annotation failed with a
/// deferrable diagnostic, while its syntax stays retained for the later strict pass.
///
/// Unknown nominal names have no TypeId yet. A collection with an unresolved capacity still has
/// a useful element type, so use its growable shape for shell/default parsing and let the later
/// strict pass resolve the authored capacity. The caller has already resolved `type_annotation`
/// in this context, so only its collection element is worth resolving again.
fn resolve_signature_type_fallback(
    type_annotation: &ParsedTypeRef,
    span: Option<SourceSpan>,
    expression_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    fallback_policy: SignatureTypeFallbackPolicy,
) -> SignatureResult<Option<TypeId>> {
    let ParsedTypeRef::Collection { element, .. } = type_annotation else {
        return Ok(None);
    };
    let element_type_id = match resolve_signature_type_annotation(
        element,
        span,
        expression_context,
        type_interner,
        const_values,
        string_table,
    ) {
        Ok(annotation) => annotation.type_id,
        Err(ExpressionParseError::Diagnostic(diagnostic))
            if should_fallback_signature_type(&diagnostic, fallback_policy) =>
        {
            resolve_signature_type_fallback(
                element,
                span,
                expression_context,
                type_interner,
                const_values,
                string_table,
                fallback_policy,
            )?
        }
        Err(error) => return Err(error),
    };
    let Some(element_type_id) = element_type_id else {
        return Ok(None);
    };
    Ok(Some(
        type_interner
            .environment_mut_for_derived_types()
            .intern_collection(element_type_id, None),
    ))
}

/// Parse the default-value expression for a single signature parameter.
#[allow(
    clippy::too_many_arguments,
    reason = "default parsing keeps the member, owner, type, scope and mutable state as separate inputs"
)]
fn parse_signature_default_expression(
    member: &SignatureMemberSyntax,
    source_owner: &AstCursor,
    type_id: TypeId,
    expression_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> SignatureResult<OwnedExpr> {
    let mut parameter_context = expression_context.to_owned();
    parameter_context.expected_result_type_ids = vec![type_id];

    let mut expected_type = parse_expectation_for_type_id(type_id, type_interner.environment());
    let mut cast_target_context = cast_target_context_for_type_id(
        type_id,
        type_interner.environment(),
        string_table,
        &*path_fork,
    );
    // The default range is canonical source provenance. Nest an explicit cursor over the live
    // owner so bounded expression handoffs retain canonical ownership; the canonical view reports
    // EOF at the range end and excludes that synthetic EOF from canonical positions.
    let range = member.default_range.ok_or_else(|| {
        CompilerError::compiler_error("signature default expression range was unexpectedly empty")
    })?;
    let mut expression_stream = source_owner.nested_cursor(range).map_err(|error| {
        CompilerError::compiler_error(format!(
            "signature default expression range is outside its source owner: {error:?}"
        ))
    })?;

    let input = ExpressionParseInput::new(
        ExpressionParseResources {
            token_stream: &mut expression_stream,
            scope_context: &parameter_context,
            type_interner,
            expression_builder,
            const_values,
            expected_type: &mut expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode: &member.value_mode,
            string_table,
            path_fork,
        },
        ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: true,
            allow_boundary_catch: true,
            allow_expected_result_evidence: true,
        },
    );
    create_expression_with_trailing_newline_policy(input)
}

#[cfg(test)]
#[path = "tests/function_parsing_tests.rs"]
mod function_parsing_tests;

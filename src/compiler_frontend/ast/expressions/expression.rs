//! AST expression values and constructor helpers used before HIR lowering.
//!
//! WHAT: defines frontend expression kinds plus the factory methods that build typed AST values.
//! WHY: parser and folding code should create expressions through one readable surface instead of
//! manually reassembling `Expression` fields at each call site.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
pub use crate::compiler_frontend::ast::expressions::expression_kind::{
    ExpressionKind, MapLiteralEntry, Operator,
};
pub use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, PlaceExpression,
};
#[cfg(test)]
pub use crate::compiler_frontend::ast::expressions::expression_types::FallibleCarrierVariant;
pub use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, TemplateConstValueKind,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::{DataType, ReceiverKey, diagnostic_type_spelling};
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

/// The kind determines the runtime shape and constant-foldability of the value.
///
/// `Runtime` carries a small AST fragment for expressions that could not be folded.
/// `ExpressionKind` mirrors core datatypes that produce values, omitting structural
/// types that do not represent first-class runtime values.
#[derive(Clone, Debug)]
pub struct Expression {
    pub kind: ExpressionKind,
    /// Canonical `TypeId` for the expression's resolved semantic type.
    ///
    /// WHAT: expression factories seed builtin scalar IDs immediately; callers that build
    ///       nominal, constructed, or function values must provide/resolve the canonical ID
    ///       while they still hold the AST type interner.
    /// WHY: HIR expression lowering consumes these IDs directly from the final module
    ///      `TypeEnvironment`; AST finalization only debug-validates that no orphan IDs remain.
    pub type_id: TypeId,
    /// Non-authoritative type spelling kept for diagnostics and parse-era declarations.
    ///
    /// WHAT: preserves the written type spelling for diagnostics and debug output.
    /// WHY: semantic identity for executable expressions is `type_id`; this field is
    ///      display-only and must never be used for semantic type decisions.
    pub diagnostic_type: DataType,
    /// Receiver metadata for function declarations.
    ///
    /// WHAT: records whether an `ExpressionKind::Function` was declared with a
    /// receiver parameter.
    /// WHY: receiver lookup must distinguish free functions from receiver
    /// methods without inspecting diagnostic-only `DataType` structure.
    pub function_receiver: Option<ReceiverKey>,
    /// Frontend access classification for this expression.
    ///
    /// WHY: mutability and reference semantics are tracked separately from the
    ///      diagnostic type so that lowering stages can make ownership decisions.
    pub value_mode: ValueMode,
    /// Source location where this expression was parsed.
    pub location: SourceLocation,
    /// Reactive source identity carried independently of the expression type.
    ///
    /// WHAT: marks declarations and parameter references that are stable reactive sources.
    /// WHY: `$T` is access syntax, not a wrapper type, so the underlying `TypeId` remains the
    /// ordinary value type while call/template validation can still require a source identity.
    pub reactive_source: Option<ReactiveSource>,
    /// Reactive template-string metadata carried independently of the expression type.
    ///
    /// WHAT: marks `String` values that are backed by a runtime template or by a direct
    /// template-value parameter passthrough.
    /// WHY: reactive templates still have ordinary semantic type `String`; backend-facing
    /// dependency information must therefore travel as value metadata rather than as a wrapper
    /// type or an inferred expression dependency graph.
    pub reactive_template: Option<ReactiveTemplateMetadata>,
    /// Explicit value-level const-record classification.
    ///
    /// WHAT: `ConstRecord` means this expression is a compile-time member group
    ///       constructed in a `#=` constant context.
    /// WHY: const-record status is a value fact, not a type identity. Semantic
    ///      decisions must inspect this field, not `diagnostic_type`.
    pub const_record_state: ConstRecordState,

    /// Tracks whether this value was derived from regular division (`/`).
    ///
    /// WHY: explicit `Int` contexts should emit a targeted diagnostic when a
    /// value comes from `/`, even when constant folding removed the original
    /// operator node.
    pub contains_regular_division: bool,

    /// Direct synthetic compile-time interface provenance for this value.
    ///
    /// WHAT: records the member-granular synthetic-interface dependencies carried by this
    /// value. Empty means portable (no project-context dependency). Non-empty provenance is a
    /// sorted, duplicate-free set that must be preserved through every value transformation
    /// that keeps or derives the value's semantic meaning, including constant folding and
    /// direct coercion/copy/pass-through paths. Dependencies originate as AST value metadata and
    /// are not inferred by reparsing source or walking display names.
    /// WHY: the per-function link-fact lane needs stable, deterministic provenance that
    /// survives AST value transformations and HIR lowering without leaking process-local IDs,
    /// source locations or interned names.
    pub synthetic_interface_provenance: SyntheticInterfaceProvenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactiveSourceKind {
    Declaration,
    Parameter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveSource {
    pub path: InternedPath,
    pub kind: ReactiveSourceKind,
}

/// A `String` parameter whose runtime value may itself be a template string.
///
/// WHAT: records the parameter identity used by a template body, for example `[content]` where
/// `content String` can receive a reactive template value from a caller.
/// WHY: V1 preserves only direct argument/return/template value flow. This placeholder lets call
/// metadata substitute the concrete argument metadata without adding closures or whole-program
/// dependency solving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveTemplateParameterDependency {
    pub parameter: InternedPath,
    pub location: SourceLocation,
}

impl ReactiveTemplateParameterDependency {
    pub fn new(parameter: InternedPath, location: SourceLocation) -> Self {
        Self {
            parameter,
            location,
        }
    }
}

/// Value-level metadata for template-backed strings.
///
/// Plain strings carry `None`. Template expressions and values that directly pass template
/// strings through ordinary `String` parameters carry `Some`, with concrete subscriptions when
/// known and parameter placeholders when the dependency is supplied by a caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReactiveTemplateMetadata {
    pub subscriptions: Vec<ReactiveSubscription>,
    pub template_value_parameters: Vec<ReactiveTemplateParameterDependency>,
    pub template_backed: bool,
}

impl ReactiveTemplateMetadata {
    pub fn template_backed() -> Self {
        Self {
            subscriptions: Vec::new(),
            template_value_parameters: Vec::new(),
            template_backed: true,
        }
    }

    pub fn from_template_value_parameter(
        parameter: InternedPath,
        location: SourceLocation,
    ) -> Self {
        let mut metadata = Self::template_backed();
        metadata.push_template_value_parameter(ReactiveTemplateParameterDependency::new(
            parameter, location,
        ));
        metadata
    }

    pub fn push_subscription(&mut self, subscription: ReactiveSubscription) {
        self.template_backed = true;
        if !self.subscriptions.contains(&subscription) {
            self.subscriptions.push(subscription);
        }
    }

    pub fn push_template_value_parameter(
        &mut self,
        dependency: ReactiveTemplateParameterDependency,
    ) {
        self.template_backed = true;
        if !self.template_value_parameters.contains(&dependency) {
            self.template_value_parameters.push(dependency);
        }
    }

    pub fn merge_from(&mut self, other: &ReactiveTemplateMetadata) {
        self.template_backed |= other.template_backed;

        for subscription in &other.subscriptions {
            self.push_subscription(subscription.clone());
        }

        for dependency in &other.template_value_parameters {
            self.push_template_value_parameter(dependency.clone());
        }
    }

    /// Returns whether this template value needs live runtime dependency handling.
    ///
    /// Template-backed strings with no subscriptions are still templates, but only concrete
    /// `$(source)` subscriptions or template-value parameter placeholders need reactive mounting
    /// and lazy snapshot lowering.
    pub fn has_runtime_dependency(&self) -> bool {
        !self.subscriptions.is_empty() || !self.template_value_parameters.is_empty()
    }

    /// Substitute direct call arguments for parameter placeholders.
    ///
    /// WHAT:
    /// - `$T` subscriptions captured inside a callee are rebound to the caller's reactive source.
    /// - Ordinary `String` parameter placeholders merge the argument's template metadata.
    ///
    /// WHY: this is the V1 direct value-flow boundary. It deliberately does not inspect arbitrary
    /// string operations or infer dependencies from expression structure.
    pub fn instantiate_for_call(
        &self,
        parameters: &[Declaration],
        arguments: &[CallArgument],
    ) -> Option<Self> {
        let mut instantiated = Self {
            subscriptions: Vec::new(),
            template_value_parameters: Vec::new(),
            template_backed: self.template_backed,
        };

        for subscription in &self.subscriptions {
            let mut resolved_subscription = subscription.clone();
            if resolved_subscription.source.kind == ReactiveSourceKind::Parameter
                && let Some(parameter_index) =
                    parameter_index_by_path(parameters, &resolved_subscription.source.path)
                && let Some(argument_source) = arguments
                    .get(parameter_index)
                    .and_then(|argument| argument.value.reactive_source.clone())
            {
                resolved_subscription.source = argument_source;
            }
            instantiated.push_subscription(resolved_subscription);
        }

        for dependency in &self.template_value_parameters {
            let Some(parameter_index) = parameter_index_by_path(parameters, &dependency.parameter)
            else {
                instantiated.push_template_value_parameter(dependency.clone());
                continue;
            };

            if let Some(argument_metadata) = arguments
                .get(parameter_index)
                .and_then(|argument| argument.value.reactive_template.as_ref())
            {
                instantiated.merge_from(argument_metadata);
            }
        }

        (instantiated.template_backed
            || !instantiated.subscriptions.is_empty()
            || !instantiated.template_value_parameters.is_empty())
        .then_some(instantiated)
    }
}

fn parameter_index_by_path(parameters: &[Declaration], path: &InternedPath) -> Option<usize> {
    parameters
        .iter()
        .position(|parameter| &parameter.id == path)
}

/// Canonical and diagnostic type data for a collection expression.
///
/// WHAT: groups the element type, optional fixed capacity, and optional exact
/// collection `TypeId` supplied by an explicit receiving context.
/// WHY: collection literal parsing needs all of these facts together, and keeping
/// them in one small input avoids long constructor argument lists.
pub(crate) struct CollectionExpressionType {
    pub(crate) element_type_id: TypeId,
    pub(crate) element_diagnostic_type: DataType,
    pub(crate) fixed_capacity: Option<usize>,
    pub(crate) collection_type_id: Option<TypeId>,
}

/// Canonical and diagnostic type data for a map expression.
///
/// WHAT: groups the key type, value type, and optional exact semantic identity
///       supplied by an explicit receiving context.
/// WHY: map literal construction needs all of these facts together, and keeping
///      them in one small input avoids long constructor argument lists.
pub(crate) struct MapLiteralExpressionType {
    pub(crate) key_type_id: TypeId,
    pub(crate) value_type_id: TypeId,
    pub(crate) key_diagnostic_type: DataType,
    pub(crate) value_diagnostic_type: DataType,
    pub(crate) map_type_id: Option<TypeId>,
}

/// Canonical type data for call expressions built after semantic resolution.
///
/// WHAT: computes the canonical `TypeId` and a display-only `DataType` from the
/// resolved return TypeIds so call constructors do not need separate diagnostic vectors.
/// WHY: typed call constructors should compute all call typing state up front instead of
/// building placeholder IDs and recomputing them later.
struct ResolvedCallTypes {
    result_type_ids: Vec<TypeId>,
    expression_type_id: TypeId,
    diagnostic_type: DataType,
}

/// Input struct for `Expression::handled_fallible_host_function_call_with_typed_arguments`
/// to avoid a long parameter list.
pub(crate) struct HandledFallibleHostFunctionCallInput {
    pub(crate) id: ExternalFunctionId,
    pub(crate) args: Vec<CallArgument>,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) error_type_id: TypeId,
    pub(crate) handling: FallibleExpressionHandling,
    pub(crate) location: SourceLocation,
}

impl ResolvedCallTypes {
    fn new(result_type_ids: Vec<TypeId>, type_environment: &mut TypeEnvironment) -> Self {
        let expression_type_id = match result_type_ids.as_slice() {
            [] => type_environment.builtins().none,
            [single] => *single,
            multiple => type_environment.intern_tuple(multiple.to_vec()),
        };
        let diagnostic_type = diagnostic_type_spelling(expression_type_id, type_environment);

        Self {
            result_type_ids,
            expression_type_id,
            diagnostic_type,
        }
    }
}

/// Best-effort bridge for parse/header paths that still start from diagnostic spelling.
///
/// WHAT: maps syntax/display-only `DataType` values to builtin TypeId hints when the caller does
///      not yet have enough context to resolve through `TypeEnvironment`.
/// WHY: this is a parse-boundary hint, not a semantic equality path. Executable AST
///      and HIR should carry canonical TypeIds resolved through the active type environment.
pub(crate) fn type_id_hint_for_diagnostic_type(data_type: &DataType) -> TypeId {
    match data_type {
        DataType::Bool | DataType::True | DataType::False => builtin_type_ids::BOOL,
        DataType::Int => builtin_type_ids::INT,
        DataType::Float => builtin_type_ids::FLOAT,
        // Decimal is intentionally inactive in the Alpha surface. The hint is
        // preserved only for diagnostic round-tripping of the inactive builtin.
        DataType::Decimal => builtin_type_ids::DECIMAL,
        DataType::StringSlice | DataType::Template | DataType::Path(_) => builtin_type_ids::STRING,
        DataType::Char => builtin_type_ids::CHAR,
        DataType::Range => builtin_type_ids::RANGE,
        DataType::None | DataType::Inferred => builtin_type_ids::NONE,
        DataType::Struct { type_id, .. } | DataType::Choices { type_id, .. } => *type_id,
        _ => builtin_type_ids::NONE,
    }
}

/// Input struct for `Expression::choice_construct` to avoid a long parameter list.
pub struct ChoiceConstructInput {
    pub nominal_path: InternedPath,
    pub tag: usize,
    pub fields: Vec<Declaration>,
    pub diagnostic_type: DataType,
    pub type_id: TypeId,
    pub location: SourceLocation,
    pub value_mode: ValueMode,
}

impl Expression {
    /// Generic constructor for an expression with the given kind and type metadata.
    pub fn new(
        kind: ExpressionKind,
        location: SourceLocation,
        type_id: TypeId,
        diagnostic_type: DataType,
        value_mode: ValueMode,
    ) -> Self {
        Self {
            type_id,
            diagnostic_type,
            function_receiver: None,
            kind,
            location,
            value_mode,
            reactive_source: None,
            reactive_template: None,
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
            synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
        }
    }

    /// Returns true if this expression represents a const-record value.
    pub fn is_const_record_value(&self) -> bool {
        matches!(self.const_record_state, ConstRecordState::ConstRecord)
    }

    /// Marks whether this expression originates from a regular division operator.
    pub fn with_regular_division_provenance(mut self, contains: bool) -> Self {
        self.contains_regular_division = contains;
        self
    }

    /// Attach direct synthetic compile-time interface provenance to this value.
    ///
    /// Compiler-internal: the future synthetic-interface producer (config/provider binding) and
    /// tests use this to inject explicit member-granular dependencies. Production AST construction
    /// leaves provenance empty because no real producer exists yet.
    pub fn with_synthetic_interface_provenance(
        mut self,
        provenance: SyntheticInterfaceProvenance,
    ) -> Self {
        self.synthetic_interface_provenance = provenance;
        self
    }

    /// Mark this expression as a reference to stable reactive storage.
    pub fn with_reactive_source(mut self, source: ReactiveSource) -> Self {
        self.reactive_source = Some(source);
        self
    }

    /// Remove reactive identity at a snapshot boundary.
    pub fn clear_reactive_source(&mut self) {
        self.reactive_source = None;
    }

    /// Returns whether the expression can satisfy a `$T` parameter or subscription source.
    pub fn is_reactive_source(&self) -> bool {
        self.reactive_source.is_some()
    }

    /// Mark this `String` expression as a template-backed value.
    pub fn with_reactive_template_metadata(mut self, metadata: ReactiveTemplateMetadata) -> Self {
        self.reactive_template = Some(metadata);
        self
    }

    /// Centralises scalar literal construction so literal factories stay structurally identical.
    fn scalar_literal(
        kind: ExpressionKind,
        type_id: TypeId,
        diagnostic_type: DataType,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        Self::new(kind, location, type_id, diagnostic_type, value_mode)
    }

    fn call_expression_with_resolved_types(
        kind: ExpressionKind,
        resolved_types: ResolvedCallTypes,
        location: SourceLocation,
    ) -> Self {
        let ResolvedCallTypes {
            result_type_ids: _,
            expression_type_id,
            diagnostic_type,
        } = resolved_types;

        Self::new(
            kind,
            location,
            expression_type_id,
            diagnostic_type,
            // Planned: derive ownership from alias-aware return signatures once
            // signature alias metadata is threaded through expression construction.
            // If the return signature is a reference (the name of a parameter passed in),
            // then this is a reference to that parameter.
            ValueMode::MutableOwned,
        )
    }

    /// Wraps an expression-owned runtime RPN stack into a single runtime expression.
    pub fn runtime_with_type_id(
        rpn: ExpressionRpn,
        data_type: DataType,
        type_id: TypeId,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        let contains_regular_division = rpn.contains_regular_division();

        debug_assert!(
            rpn.validate_no_statement_bodies(),
            "Runtime RPN must not carry statement bodies"
        );

        Self::new(
            ExpressionKind::Runtime(rpn),
            location,
            type_id,
            data_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
    }

    /// Constructs an integer literal expression.
    pub fn int(value: i32, location: SourceLocation, value_mode: ValueMode) -> Self {
        Self::scalar_literal(
            ExpressionKind::Int(value),
            builtin_type_ids::INT,
            DataType::Int,
            location,
            value_mode,
        )
    }

    /// Constructs a floating-point literal expression.
    pub fn float(value: f64, location: SourceLocation, value_mode: ValueMode) -> Self {
        Self::scalar_literal(
            ExpressionKind::Float(value),
            builtin_type_ids::FLOAT,
            DataType::Float,
            location,
            value_mode,
        )
    }

    /// Constructs a string slice literal expression.
    pub fn string_slice(value: StringId, location: SourceLocation, value_mode: ValueMode) -> Self {
        Self::scalar_literal(
            ExpressionKind::StringSlice(value),
            builtin_type_ids::STRING,
            DataType::StringSlice,
            location,
            value_mode,
        )
    }

    /// Constructs a boolean literal expression.
    pub fn bool(value: bool, location: SourceLocation, value_mode: ValueMode) -> Self {
        Self::scalar_literal(
            ExpressionKind::Bool(value),
            builtin_type_ids::BOOL,
            DataType::Bool,
            location,
            value_mode,
        )
    }

    /// Constructs a character literal expression.
    pub fn char(value: char, location: SourceLocation, value_mode: ValueMode) -> Self {
        Self::scalar_literal(
            ExpressionKind::Char(value),
            builtin_type_ids::CHAR,
            DataType::Char,
            location,
            value_mode,
        )
    }

    /// Constructs a reference expression from an interned path.
    pub fn reference_with_type_id(
        id: InternedPath,
        data_type: DataType,
        type_id: TypeId,
        location: SourceLocation,
        value_mode: ValueMode,
        const_record_state: ConstRecordState,
    ) -> Self {
        let mut expression = Self::new(
            ExpressionKind::Reference(id),
            location,
            type_id,
            data_type,
            value_mode,
        );
        expression.const_record_state = const_record_state;
        expression
    }

    /// Constructs a function expression with an optional receiver.
    pub fn function(
        receiver: Option<ReceiverKey>,
        signature: FunctionSignature,
        type_id: TypeId,
        location: SourceLocation,
    ) -> Self {
        let function_data_type = DataType::Function(Box::new(receiver.clone()), signature.clone());
        let mut expression = Self::new(
            ExpressionKind::Function(signature),
            location,
            type_id,
            function_data_type,
            ValueMode::ImmutableReference,
        );
        expression.function_receiver = receiver;
        expression
    }

    /// Constructs a resolved function call expression.
    pub(crate) fn function_call_with_typed_arguments(
        name: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::FunctionCall {
                name,
                args,
                result_type_ids,
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved receiver method call expression.
    pub(crate) fn method_call_with_typed_arguments(
        receiver: Expression,
        method_path: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::MethodCall {
                receiver: Box::new(receiver),
                method_path,
                args,
                result_type_ids,
                location: location.clone(),
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved collection builtin receiver call expression.
    pub(crate) fn collection_builtin_call_with_typed_arguments(
        receiver: Expression,
        op: CollectionBuiltinOp,
        receiver_requires_mutable: bool,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::CollectionBuiltinCall {
                receiver: Box::new(receiver),
                op,
                receiver_requires_mutable,
                args,
                result_type_ids,
                location: location.clone(),
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved map builtin receiver call expression.
    pub(crate) fn map_builtin_call_with_typed_arguments(
        receiver: Expression,
        op: MapBuiltinOp,
        receiver_requires_mutable: bool,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::MapBuiltinCall {
                receiver: Box::new(receiver),
                op,
                receiver_requires_mutable,
                args,
                result_type_ids,
                location: location.clone(),
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved fallible function call with explicit error handling.
    pub(crate) fn handled_fallible_function_call_with_typed_arguments(
        name: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        handling: FallibleExpressionHandling,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids,
                handling,
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved host function call expression.
    pub(crate) fn host_function_call_with_typed_arguments(
        id: ExternalFunctionId,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::HostFunctionCall {
                id,
                args,
                result_type_ids,
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved fallible host function call with explicit error handling.
    pub(crate) fn handled_fallible_host_function_call_with_typed_arguments(
        input: HandledFallibleHostFunctionCallInput,
        type_environment: &mut TypeEnvironment,
    ) -> Self {
        let HandledFallibleHostFunctionCallInput {
            id,
            args,
            result_type_ids,
            error_type_id,
            handling,
            location,
        } = input;
        let resolved_types = ResolvedCallTypes::new(result_type_ids, type_environment);

        let result_type_ids = resolved_types.result_type_ids.clone();
        Self::call_expression_with_resolved_types(
            ExpressionKind::HandledFallibleHostFunctionCall {
                id,
                args,
                result_type_ids,
                error_type_id,
                handling,
            },
            resolved_types,
            location,
        )
    }

    /// Constructs a resolved explicit `cast` expression.
    ///
    /// WHAT: builds the AST value for a cast whose evidence and handling have
    ///      already been validated. The resulting type is the target type (or the
    ///      optional-wrapped target type when the boundary was `T?`).
    /// WHY: boundary callers should produce one resolved `Expression` value rather
    ///      than manually assembling `ResolvedCastExpression` fields.
    pub(crate) fn cast(
        cast: ResolvedCastExpression,
        target_type_id: TypeId,
        type_environment: &TypeEnvironment,
    ) -> Self {
        let location = cast.location.clone();
        let diagnostic_type = diagnostic_type_spelling(target_type_id, type_environment);
        let value_mode = cast.source.value_mode.to_owned();
        let synthetic_interface_provenance = cast.source.synthetic_interface_provenance.clone();

        let mut expression = Self::new(
            ExpressionKind::Cast(cast),
            location,
            target_type_id,
            diagnostic_type,
            value_mode,
        );
        expression.synthetic_interface_provenance = synthetic_interface_provenance;
        expression
    }

    /// Build an explicit contextual coercion node.
    ///
    /// WHAT: wraps `value` in a `Coerced` expression kind that carries the
    /// target type explicitly in the AST.
    /// WHY: contextual conversions such as `Int` → `Float` and `T` → `T?` must
    /// be represented deliberately so lowering stages can emit the correct
    /// conversion rather than silently mistyping the inner value.
    pub fn coerced(value: Expression, to_type: TypeId) -> Self {
        let location = value.location.clone();
        let value_mode = value.value_mode.to_owned();
        let reactive_source = value.reactive_source.clone();
        let reactive_template = value.reactive_template.clone();
        let contains_regular_division = value.contains_regular_division;
        let const_record_state = value.const_record_state;
        let synthetic_interface_provenance = value.synthetic_interface_provenance.clone();
        let mut expression = Self::new(
            ExpressionKind::Coerced {
                value: Box::new(value),
                to_type,
            },
            location,
            to_type,
            // diagnostic_type is non-authoritative; Inferred is sufficient for a
            // coercion node because semantic identity comes from type_id alone.
            DataType::Inferred,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division);
        expression.const_record_state = const_record_state;
        expression.reactive_source = reactive_source;
        expression.reactive_template = reactive_template;
        expression.synthetic_interface_provenance = synthetic_interface_provenance;
        expression
    }

    /// Constructs a fallible carrier expression for lowering fixtures.
    #[cfg(test)]
    pub fn result_construct_with_type_id(
        variant: FallibleCarrierVariant,
        value: Expression,
        diagnostic_type: DataType,
        type_id: TypeId,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        let contains_regular_division = value.contains_regular_division;
        let synthetic_interface_provenance = value.synthetic_interface_provenance.clone();
        Self::new(
            ExpressionKind::FallibleCarrierConstruct {
                variant,
                value: Box::new(value),
            },
            location,
            type_id,
            diagnostic_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance)
    }

    /// Wraps a fallible expression with explicit handling.
    pub fn handled_result_with_type_id(
        value: Expression,
        handling: FallibleExpressionHandling,
        result_type_id: TypeId,
        diagnostic_type: DataType,
        location: SourceLocation,
    ) -> Self {
        let contains_regular_division = value.contains_regular_division;
        let synthetic_interface_provenance = value.synthetic_interface_provenance.clone();
        Self::new(
            ExpressionKind::HandledFallibleExpression {
                value: Box::new(value),
                handling,
            },
            location,
            result_type_id,
            diagnostic_type,
            ValueMode::ImmutableOwned,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance)
    }

    /// Wraps an optional expression with postfix `?` propagation.
    pub fn option_propagation_with_type_id(
        value: Expression,
        inner_type_id: TypeId,
        diagnostic_type: DataType,
        location: SourceLocation,
    ) -> Self {
        let contains_regular_division = value.contains_regular_division;
        let synthetic_interface_provenance = value.synthetic_interface_provenance.clone();
        Self::new(
            ExpressionKind::OptionPropagation {
                value: Box::new(value),
            },
            location,
            inner_type_id,
            diagnostic_type,
            ValueMode::ImmutableOwned,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance)
    }

    /// Constructs a collection expression with a resolved element type.
    pub(crate) fn collection_with_type_id(
        items: Vec<Expression>,
        collection_type: CollectionExpressionType,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        let collection_type_id = collection_type.collection_type_id.unwrap_or_else(|| {
            type_environment.intern_collection(
                collection_type.element_type_id,
                collection_type.fixed_capacity,
            )
        });
        let contains_regular_division = items.iter().any(|item| item.contains_regular_division);
        let synthetic_interface_provenance = SyntheticInterfaceProvenance::union_all(
            items
                .iter()
                .map(|item| &item.synthetic_interface_provenance),
        );
        // `diagnostic_type` is display-only; semantic identity comes from `collection_type_id`.
        let diagnostic_type = match collection_type.fixed_capacity {
            Some(capacity) => {
                DataType::fixed_collection(collection_type.element_diagnostic_type, capacity)
            }
            None => DataType::collection(collection_type.element_diagnostic_type),
        };
        Self::new(
            ExpressionKind::Collection(items),
            location,
            collection_type_id,
            diagnostic_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance)
    }

    /// Constructs a map literal expression with resolved key and value types.
    pub(crate) fn map_literal_with_type_id(
        entries: Vec<MapLiteralEntry>,
        map_type: MapLiteralExpressionType,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        let map_type_id = map_type.map_type_id.unwrap_or_else(|| {
            type_environment.intern_map(map_type.key_type_id, map_type.value_type_id)
        });
        let contains_regular_division = entries.iter().any(|entry| {
            entry.key.contains_regular_division || entry.value.contains_regular_division
        });
        let synthetic_interface_provenance =
            SyntheticInterfaceProvenance::union_all(entries.iter().flat_map(|entry| {
                [
                    &entry.key.synthetic_interface_provenance,
                    &entry.value.synthetic_interface_provenance,
                ]
            }));
        let diagnostic_type =
            DataType::map(map_type.key_diagnostic_type, map_type.value_diagnostic_type);
        Self::new(
            ExpressionKind::MapLiteral(entries),
            location,
            map_type_id,
            diagnostic_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance)
    }

    /// Constructs a struct instance expression.
    pub fn struct_instance(
        nominal_path: InternedPath,
        args: Vec<Declaration>,
        location: SourceLocation,
        value_mode: ValueMode,
        const_record: bool,
        generic_instance_key: Option<GenericInstantiationKey>,
        type_id: TypeId,
    ) -> Self {
        let contains_regular_division = args.iter().any(|arg| arg.value.contains_regular_division);
        let synthetic_interface_provenance = SyntheticInterfaceProvenance::union_all(
            args.iter()
                .map(|argument| &argument.value.synthetic_interface_provenance),
        );
        let struct_type = if let Some(key) = generic_instance_key {
            DataType::Struct {
                nominal_path: nominal_path.clone(),
                type_id,
                const_record,
                generic_instance_key: Some(key),
            }
        } else if const_record {
            DataType::const_struct_record(nominal_path, type_id)
        } else {
            DataType::runtime_struct(nominal_path, type_id)
        };
        let mut expression = Self::new(
            ExpressionKind::StructInstance(args),
            location,
            type_id,
            struct_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance);
        if const_record {
            expression.const_record_state = ConstRecordState::ConstRecord;
        }
        expression
    }

    /// Constructs a struct definition expression.
    pub fn struct_definition(
        args: Vec<Declaration>,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        Self::new(
            ExpressionKind::StructDefinition(args),
            location,
            builtin_type_ids::NONE,
            DataType::Inferred,
            value_mode,
        )
    }

    /// Constructs a template expression without provisional reactive metadata.
    ///
    /// WHAT: records the template value while leaving
    /// `reactive_template` unset.
    /// WHY: AST finalization owns the module store and recomputes authoritative
    /// metadata through TIR before normalization and HIR lowering.
    pub fn template(template: Template, value_mode: ValueMode) -> Self {
        let location = template.location.to_owned();
        Self::new(
            ExpressionKind::Template(Box::new(template)),
            location,
            builtin_type_ids::STRING,
            DataType::Template,
            value_mode,
        )
    }

    /// Constructs the final AST-owned payload for an ordinary runtime template.
    ///
    /// WHAT: records the neutral owned handoff shape directly on the expression while preserving
    /// the existing `String`/template value metadata used by callers.
    /// WHY: Phase 11 introduces the final AST shape before consumer cutover, so construction is
    /// explicit and testable without changing HIR lowering behavior.
    #[allow(
        dead_code,
        reason = "Phase 11 introduces the final expression shape before finalization cutover wires production callers"
    )]
    pub(crate) fn runtime_template_handoff(
        handoff: OwnedRuntimeTemplateHandoff,
        value_mode: ValueMode,
    ) -> Self {
        let location = handoff.location.to_owned();
        let mut expression = Self::new(
            ExpressionKind::RuntimeTemplateHandoff(Box::new(handoff)),
            location,
            builtin_type_ids::STRING,
            DataType::Template,
            value_mode,
        );
        expression.reactive_template = Some(ReactiveTemplateMetadata::template_backed());
        expression
    }

    /// Constructs the final AST-owned payload for a runtime slot application.
    ///
    /// WHAT: stores routed slot application data as neutral owned AST payload.
    /// WHY: later HIR cutover can lower slot applications from this variant without reaching
    /// through `Template::runtime_slot_handoff` or any TIR-internal reference.
    #[allow(
        dead_code,
        reason = "Phase 11 introduces the final expression shape before finalization cutover wires production callers"
    )]
    pub(crate) fn runtime_slot_application_handoff(
        handoff: OwnedRuntimeSlotApplicationHandoff,
        value_mode: ValueMode,
    ) -> Self {
        let location = handoff.location.to_owned();
        let mut expression = Self::new(
            ExpressionKind::RuntimeSlotApplicationHandoff(Box::new(handoff)),
            location,
            builtin_type_ids::STRING,
            DataType::Template,
            value_mode,
        );
        expression.reactive_template = Some(ReactiveTemplateMetadata::template_backed());
        expression
    }

    /// Constructs a copy expression from an AST node place.
    /// Constructs a copy expression from a frontend place expression.
    pub fn copy_with_type_id(
        place: PlaceExpression,
        data_type: DataType,
        type_id: TypeId,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        Self::new(
            ExpressionKind::Copy(place),
            location,
            type_id,
            data_type,
            value_mode.as_owned(),
        )
    }

    /// Internal sentinel used for declarations/signature defaults that do not
    /// provide a value expression in source.
    pub fn no_value(location: SourceLocation, data_type: DataType, value_mode: ValueMode) -> Self {
        let type_id = type_id_hint_for_diagnostic_type(&data_type);
        Self::no_value_with_type_id(location, data_type, type_id, value_mode)
    }

    /// Internal sentinel for declarations whose canonical type is already known.
    ///
    /// Constructed and nominal types cannot be recovered from diagnostic spelling alone, so parser
    /// sites that already resolved a `TypeId` should preserve it here for later expression typing.
    pub fn no_value_with_type_id(
        location: SourceLocation,
        data_type: DataType,
        type_id: TypeId,
        value_mode: ValueMode,
    ) -> Self {
        Self::new(
            ExpressionKind::NoValue,
            location,
            type_id,
            data_type,
            value_mode,
        )
    }

    /// Constructs a `none` literal for an optional type.
    pub fn option_none_with_type_id(
        inner_type_id: TypeId,
        inner_diagnostic_type: DataType,
        type_environment: &mut TypeEnvironment,
        location: SourceLocation,
    ) -> Self {
        let option_type_id = type_environment.intern_option(inner_type_id);
        // `diagnostic_type` is display-only; semantic identity comes from `option_type_id`.
        Self::new(
            ExpressionKind::OptionNone,
            location,
            option_type_id,
            DataType::Option(Box::new(inner_diagnostic_type)),
            ValueMode::ImmutableOwned,
        )
    }

    /// Constructs a choice variant instance.
    pub fn choice_construct(input: ChoiceConstructInput) -> Self {
        let contains_regular_division = input
            .fields
            .iter()
            .any(|field| field.value.contains_regular_division);
        let synthetic_interface_provenance = SyntheticInterfaceProvenance::union_all(
            input
                .fields
                .iter()
                .map(|field| &field.value.synthetic_interface_provenance),
        );
        Self::new(
            ExpressionKind::ChoiceConstruct {
                nominal_path: input.nominal_path,
                tag: input.tag,
                fields: input.fields,
            },
            input.location,
            input.type_id,
            input.diagnostic_type,
            input.value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
        .with_synthetic_interface_provenance(synthetic_interface_provenance)
    }

    /// Returns true if this expression represents a function declaration that has
    /// a receiver parameter (`this` or `This`).
    pub(crate) fn is_receiver_function(&self) -> bool {
        self.function_receiver.is_some()
    }

    /// Classifies expression constness through a caller-owned template authority.
    ///
    /// WHAT: keeps ordinary expression-shape recursion in one owner while the
    ///       caller supplies the stage-appropriate classification for template
    ///       payloads.
    /// WHY: finalization already owns exact effective TIR views,
    ///      while earlier parser callers still materialize current TIR. Both
    ///      paths must preserve identical non-template const semantics without
    ///      duplicating the expression walk.
    pub(crate) fn const_value_kind_with_template_classifier(
        &self,
        classify_template: &mut impl FnMut(&Template) -> Result<TemplateConstValueKind, TemplateError>,
    ) -> Result<ConstValueKind, TemplateError> {
        let kind = match &self.kind {
            // Literal scalars are always compile-time constants.
            ExpressionKind::Int(_)
            | ExpressionKind::Float(_)
            | ExpressionKind::StringSlice(_)
            | ExpressionKind::Bool(_)
            | ExpressionKind::Char(_) => ConstValueKind::Literal,

            #[cfg(test)]
            ExpressionKind::Path(_) => ConstValueKind::Literal,

            // Composite values are constant only when every sub-field is constant.
            ExpressionKind::ChoiceConstruct { fields, .. } => {
                if fields.is_empty() {
                    ConstValueKind::Literal
                } else if Self::declarations_are_constant_with_template_classifier(
                    fields,
                    classify_template,
                )? {
                    ConstValueKind::Composite
                } else {
                    ConstValueKind::NonConst
                }
            }

            ExpressionKind::Collection(items) => {
                if Self::expressions_are_constant_with_template_classifier(
                    items,
                    classify_template,
                )? {
                    ConstValueKind::Composite
                } else {
                    ConstValueKind::NonConst
                }
            }

            ExpressionKind::MapLiteral(_) => {
                // Map literals are intentionally not compile-time foldable in V1.
                ConstValueKind::NonConst
            }

            ExpressionKind::StructInstance(fields) => {
                if Self::declarations_are_constant_with_template_classifier(
                    fields,
                    classify_template,
                )? {
                    ConstValueKind::Composite
                } else {
                    ConstValueKind::NonConst
                }
            }

            ExpressionKind::Range(start, end) => {
                if start
                    .const_value_kind_with_template_classifier(classify_template)?
                    .is_compile_time_value()
                    && end
                        .const_value_kind_with_template_classifier(classify_template)?
                        .is_compile_time_value()
                {
                    ConstValueKind::Composite
                } else {
                    ConstValueKind::NonConst
                }
            }

            // The caller supplies the template authority for its compiler stage.
            ExpressionKind::Template(template) => {
                let template_kind = classify_template(template)?;
                Self::const_value_kind_from_template_kind(template_kind)
            }

            // Fallible carriers preserve const-ness of the wrapped value.
            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { value, .. } => {
                if value
                    .const_value_kind_with_template_classifier(classify_template)?
                    .is_compile_time_value()
                {
                    ConstValueKind::Composite
                } else {
                    ConstValueKind::NonConst
                }
            }

            // Everything else is non-const by default.
            ExpressionKind::Reference(_)
            | ExpressionKind::Copy(_)
            | ExpressionKind::Runtime(_)
            | ExpressionKind::RuntimeTemplateHandoff(_)
            | ExpressionKind::RuntimeSlotApplicationHandoff(_)
            | ExpressionKind::Function(..)
            | ExpressionKind::FunctionCall { .. }
            | ExpressionKind::Cast { .. }
            | ExpressionKind::HandledFallibleExpression { .. }
            | ExpressionKind::OptionPropagation { .. }
            | ExpressionKind::HandledFallibleFunctionCall { .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { .. }
            | ExpressionKind::HostFunctionCall { .. }
            | ExpressionKind::StructDefinition(..)
            | ExpressionKind::FieldAccess { .. }
            | ExpressionKind::MethodCall { .. }
            | ExpressionKind::CollectionBuiltinCall { .. }
            | ExpressionKind::MapBuiltinCall { .. }
            | ExpressionKind::NoValue
            | ExpressionKind::OptionNone
            | ExpressionKind::ValueBlock { .. } => ConstValueKind::NonConst,

            // Delegate const classification to the wrapped value — the coercion
            // does not change whether an expression is compile-time foldable.
            ExpressionKind::Coerced { value, .. } => {
                value.const_value_kind_with_template_classifier(classify_template)?
            }
        };

        Ok(kind)
    }

    fn expressions_are_constant_with_template_classifier(
        expressions: &[Expression],
        classify_template: &mut impl FnMut(&Template) -> Result<TemplateConstValueKind, TemplateError>,
    ) -> Result<bool, TemplateError> {
        for expression in expressions {
            if !expression
                .const_value_kind_with_template_classifier(classify_template)?
                .is_compile_time_value()
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn declarations_are_constant_with_template_classifier(
        declarations: &[Declaration],
        classify_template: &mut impl FnMut(&Template) -> Result<TemplateConstValueKind, TemplateError>,
    ) -> Result<bool, TemplateError> {
        for declaration in declarations {
            if !declaration
                .value
                .const_value_kind_with_template_classifier(classify_template)?
                .is_compile_time_value()
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn const_value_kind_from_template_kind(
        template_kind: TemplateConstValueKind,
    ) -> ConstValueKind {
        match template_kind {
            TemplateConstValueKind::RenderableString => ConstValueKind::RenderableTemplate,
            TemplateConstValueKind::LoopControlSignal => ConstValueKind::Composite,
            TemplateConstValueKind::WrapperTemplate => ConstValueKind::TemplateWrapper,
            TemplateConstValueKind::SlotInsertHelper => ConstValueKind::SlotInsertTemplate,
            TemplateConstValueKind::NonConst => ConstValueKind::NonConst,
        }
    }
}

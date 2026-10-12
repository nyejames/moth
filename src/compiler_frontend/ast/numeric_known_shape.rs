//! Construction-only known shapes for retained collection and map literals.
//!
//! WHAT: a body-owned, index-addressed description of the numeric structure of one literal or
//! binding. Leaves are either concrete canonical `TypeId`s or numeric classes that have not yet
//! received a canonical type.
//! WHY: a retained literal can only pick its element and key/value leaf types after every
//! constraint from its receiver, its named peers and its sibling nested shapes is known. Holding
//! the structure as dense IDs in `NumericConstructionState` lets a root ID be copied freely
//! without cloning descriptor graphs per binding or use.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::field_access::retained_operand_result_shape;
use crate::compiler_frontend::ast::numeric_construction::{
    ConstructionResult, NumericClassId, NumericConstructionState, NumericFamily, NumericPin,
    binding_class, binding_reference_shape, family_receives_recipe, item_is_literal_recipe_shape,
    merged_receiver_type, numeric_witness_type, raw_fragment_result_is_independently_fixed,
    retained_known_shape, single_direct_pending_reference,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::GenericTypeBindings;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId, builtin_type_ids};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use rustc_hash::{FxHashMap, FxHashSet};

/// Dense identity of one construction-only known shape node.
///
/// WHAT: an index into `NumericConstructionState`'s shape table.
/// WHY: IDs are `Copy`, so recipes, bindings and deferred literals can hold them without owning
/// or cloning any descriptor graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KnownShapeId(u32);

impl KnownShapeId {
    /// Wrap a dense index as a shape ID, rejecting indices that do not fit the ID space.
    pub(crate) fn from_dense_index(index: usize) -> Result<Self, CompilerError> {
        let raw = u32::try_from(index).map_err(|_| {
            CompilerError::compiler_error("numeric known-shape table exhausted its 32-bit ID space")
        })?;
        Ok(Self(raw))
    }

    pub(crate) fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// One known shape node.
///
/// WHAT: `Concrete` carries an exact canonical type that needs no further resolution. `Numeric`
/// names a class whose canonical leaf type is decided at completion. `Collection`, `Map` and
/// `Optional` reference their child nodes by ID.
/// WHY: a nominal or fixed-type leaf keeps its exact `TypeId`; numeric leaves stay open until
/// every constraint across the literal and its peers has been collected.
#[derive(Debug, Clone, Copy)]
pub(crate) enum KnownShape {
    Concrete {
        type_id: TypeId,
        span: Option<SourceSpan>,
    },
    Numeric(NumericClassId),
    Collection {
        element: KnownShapeId,
        fixed_capacity: Option<usize>,
    },
    Map {
        key: KnownShapeId,
        value: KnownShapeId,
    },
    /// An optional envelope over one inner shape. The canonical `T?` is interned only at
    /// materialisation, so a numeric inner leaf stays open until completion.
    Optional {
        inner: KnownShapeId,
    },
    /// A declared generic signature type whose parameters are bound to shape nodes.
    ///
    /// WHAT: `declared_type_id` is the declared type as the signature names it, such as
    /// `Packet of T`. `bindings` covers exactly the parameters that declared type mentions.
    /// WHY: a numeric origin reached through a generic result keeps its declared structure, so it
    /// materialises through `substitute_type_id` only after every linked child is concrete.
    /// Parameters the declared type never names stay out of the range.
    DeclaredSubstitution {
        declared_type_id: TypeId,
        bindings: KnownShapeBindingRange,
    },
}

/// One parameter-to-shape edge of a declared substitution.
///
/// WHAT: names one generic parameter of the declared type and the shape node bound to it.
/// WHY: edges pair by parameter identity, so unions and refusal checks never match by position.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KnownShapeBinding {
    pub(crate) parameter: GenericParameterId,
    pub(crate) shape: KnownShapeId,
}

/// Half-open range of `KnownShapeBinding` entries in the body's declared-binding table.
///
/// WHAT: `start..end` indexes `NumericConstructionState`'s declared-binding storage.
/// WHY: a `Copy` range keeps `KnownShape` free of owned descriptor graphs and per-read clones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KnownShapeBindingRange {
    start: u32,
    end: u32,
}

impl KnownShapeBindingRange {
    /// Wrap a `start..end` pair. The caller checks both bounds against the table first.
    pub(crate) fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub(crate) fn indices(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// One operation's DAG results. A class merge can change the admission policy of
/// previously visited concrete peers, so only mutation-free subwalks are reusable.
#[derive(Default)]
struct ShapePairMemo {
    pairs: FxHashMap<(KnownShapeId, KnownShapeId), bool>,
    patterns: FxHashMap<(TypeId, KnownShapeBindingRange, KnownShapeId), bool>,
    projections: FxHashMap<
        (
            TypeId,
            KnownShapeBindingRange,
            TypeId,
            KnownShapeBindingRange,
        ),
        bool,
    >,
    epoch: usize,
    // Terminal comparisons keep all maps empty until a composite edge is actually followed.
    descent_started: bool,
}

impl ShapePairMemo {
    fn invalidate(&mut self) {
        self.epoch += 1;
        self.pairs.clear();
        self.patterns.clear();
        self.projections.clear();
    }
}

/// Register the exact receiver witnesses implied by a shape against a receiver type.
///
/// WHAT: walks the shape alongside the receiver. Each present-value position peels at most one
/// option receiver layer for its own contextual wrap: an Optional origin for its own layer, and a
/// numeric leaf, collection, map or declared substitution for the value it holds. A numeric leaf
/// records one witness when the peeled receiver is a scalar fixed numeric type. A pinned origin
/// admits only a receiver that `family_receives_recipe` accepts for its family. Any other numeric
/// type is recorded as a refused peer, so it never retypes the origin. Concrete leaves already
/// carry their type and record nothing.
/// WHY: witnesses must come from the receiver's exact structure, never from a default. Explicit
/// receivers, forward results and union peers share this one walk, so Option and collection
/// peeling never diverge. A witness registered before the pin is unaffected, so an immediate
/// annotation still materialises its literal directly. Recursing directly on the state avoids
/// staging witnesses in a per-use buffer.
pub(crate) fn register_shape_witness(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    shape: KnownShapeId,
    receiver: TypeId,
    span: Option<SourceSpan>,
) {
    let authored_order = state.take_authored_order();
    register_ordered_shape_witness(state, environment, shape, receiver, span, authored_order);
}

/// Project a receiver while preserving the parse-time key of its authored source.
///
/// WHY: forward completion and concrete shape joins can run after later receiving uses.
///      Reusing their provenance keeps diagnostics independent of completion and union order.
pub(crate) fn register_ordered_shape_witness(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    shape: KnownShapeId,
    receiver: TypeId,
    span: Option<SourceSpan>,
    authored_order: u64,
) {
    let mut visited = FxHashSet::default();
    register_ordered_shape_witness_inner(
        state,
        environment,
        shape,
        receiver,
        span,
        authored_order,
        &mut visited,
    );
}

fn register_ordered_shape_witness_inner(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    shape: KnownShapeId,
    receiver: TypeId,
    span: Option<SourceSpan>,
    authored_order: u64,
    visited: &mut FxHashSet<(KnownShapeId, TypeId)>,
) {
    let visit_key = (shape, receiver);
    match state.shape(shape) {
        KnownShape::Concrete { .. } => {}
        KnownShape::Numeric(class) => {
            // Scalar receiving sites need no traversal storage. Composite walks already own
            // marks, so shared numeric descendants still register only once per receiver.
            if !visited.is_empty() && !visited.insert(visit_key) {
                return;
            }
            // A present numeric leaf wraps into one receiver optional layer. An Optional origin has
            // already consumed its own layer above, so this peels only a layer the leaf still needs.
            let receiver = environment.option_inner_type(receiver).unwrap_or(receiver);
            let Some(type_id) = numeric_witness_type(receiver, environment) else {
                return;
            };
            if family_receives_recipe(state.class_pinned_family(class), receiver, environment) {
                state.add_ordered_receiver_witness(
                    class,
                    type_id,
                    span,
                    authored_order,
                    environment,
                );
            } else {
                state.refuse_ordered_pinned_peer(class, type_id, span, authored_order);
            }
        }
        KnownShape::Collection { element, .. } => {
            let receiver = environment.option_inner_type(receiver).unwrap_or(receiver);
            if let Some(collection) = environment.collection_shape(receiver) {
                if !visited.insert(visit_key) {
                    return;
                }
                register_ordered_shape_witness_inner(
                    state,
                    environment,
                    element,
                    collection.element_type,
                    span,
                    authored_order,
                    visited,
                );
            }
        }
        KnownShape::Map { key, value } => {
            let receiver = environment.option_inner_type(receiver).unwrap_or(receiver);
            if let Some(map) = environment.map_shape(receiver) {
                if !visited.insert(visit_key) {
                    return;
                }
                register_ordered_shape_witness_inner(
                    state,
                    environment,
                    key,
                    map.key_type,
                    span,
                    authored_order,
                    visited,
                );
                register_ordered_shape_witness_inner(
                    state,
                    environment,
                    value,
                    map.value_type,
                    span,
                    authored_order,
                    visited,
                );
            }
        }
        KnownShape::Optional { inner } => {
            if !visited.insert(visit_key) {
                return;
            }
            let receiver = environment.option_inner_type(receiver).unwrap_or(receiver);
            register_ordered_shape_witness_inner(
                state,
                environment,
                inner,
                receiver,
                span,
                authored_order,
                visited,
            );
        }
        KnownShape::DeclaredSubstitution {
            declared_type_id,
            bindings,
        } => {
            // An optional receiver wraps the declared instance, so its payload is what projects.
            let receiver = environment.option_inner_type(receiver).unwrap_or(receiver);
            let Some(projected) = declared_projection(environment, declared_type_id, receiver)
            else {
                return;
            };
            let mut descending = false;
            for index in bindings.indices() {
                let edge = state.declared_binding(index);
                if let Some(parameter_type) = projected.replacement_map().get(&edge.parameter) {
                    if !descending {
                        if !visited.insert(visit_key) {
                            return;
                        }
                        descending = true;
                    }
                    register_ordered_shape_witness_inner(
                        state,
                        environment,
                        edge.shape,
                        *parameter_type,
                        span,
                        authored_order,
                        visited,
                    );
                }
            }
        }
    }
}

/// Join the structure of two shapes into one equality structure.
///
/// WHAT: numeric leaves union their classes unless the peer-family rule refuses the pair. A
/// concrete leaf contributes its exact type as a natural peer witness on the other side's numeric
/// leaves, so a pinned bound origin admits only a receiver its family accepts. Mismatched structure
/// is left for the coercion path to report.
/// WHY: element peers and sibling nested shapes must share leaf classes before any default is
/// chosen, and the public union enforces the same peer rules as its collection caller. Returns
/// `false` when at least one numeric pair was refused; a refused pair stays separate.
pub(crate) fn union_shape_classes(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeId,
    right: KnownShapeId,
) -> bool {
    let mut memo = ShapePairMemo::default();
    union_shape_classes_inner(state, environment, left, right, &mut memo)
}

fn union_shape_classes_inner(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeId,
    right: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    match (state.shape(left), state.shape(right)) {
        (KnownShape::Collection { .. }, KnownShape::Collection { .. })
        | (KnownShape::Map { .. }, KnownShape::Map { .. })
        | (KnownShape::Optional { .. }, KnownShape::Optional { .. })
        | (KnownShape::DeclaredSubstitution { .. }, KnownShape::DeclaredSubstitution { .. }) => {}
        // Only recursive comparisons benefit from memo storage. Scalar unions and mismatches
        // retain the allocation-free path used by ordinary collection elements.
        _ => return union_shape_classes_walk(state, environment, left, right, memo),
    }
    let key = (left, right);
    if let Some(result) = memo.pairs.get(&key) {
        return *result;
    }
    let epoch = memo.epoch;
    let result = union_shape_classes_walk(state, environment, left, right, memo);
    if memo.descent_started && memo.epoch == epoch {
        memo.pairs.insert(key, result);
    }
    result
}

fn union_shape_classes_walk(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeId,
    right: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    match (state.shape(left), state.shape(right)) {
        (KnownShape::Numeric(left_class), KnownShape::Numeric(right_class)) => {
            if numeric_classes_join_refused(state, left_class, right_class) {
                return false;
            }
            if !state.classes_share_root(left_class, right_class) {
                state.union_classes(left_class, right_class);
                memo.invalidate();
            }
            true
        }
        (
            KnownShape::Collection {
                element: left_element,
                ..
            },
            KnownShape::Collection {
                element: right_element,
                ..
            },
        ) => {
            memo.descent_started = true;
            union_shape_classes_inner(state, environment, left_element, right_element, memo)
        }
        (
            KnownShape::Map {
                key: left_key,
                value: left_value,
            },
            KnownShape::Map {
                key: right_key,
                value: right_value,
            },
        ) => {
            memo.descent_started = true;
            let keys = union_shape_classes_inner(state, environment, left_key, right_key, memo);
            let values =
                union_shape_classes_inner(state, environment, left_value, right_value, memo);
            keys && values
        }
        (
            KnownShape::Optional { inner: left_inner },
            KnownShape::Optional { inner: right_inner },
        ) => {
            memo.descent_started = true;
            union_shape_classes_inner(state, environment, left_inner, right_inner, memo)
        }
        (
            KnownShape::DeclaredSubstitution {
                declared_type_id: left_declared,
                bindings: left_bindings,
            },
            KnownShape::DeclaredSubstitution {
                declared_type_id: right_declared,
                bindings: right_bindings,
            },
        ) => {
            if left_declared == right_declared {
                union_declared_bindings(state, environment, left_bindings, right_bindings, memo)
            } else {
                union_declared_envelopes(
                    state,
                    environment,
                    left_declared,
                    left_bindings,
                    right_declared,
                    right_bindings,
                    memo,
                )
            }
        }
        (KnownShape::Concrete { type_id, span }, _) => {
            let order = state.shape_authored_order(left);
            register_ordered_shape_witness(state, environment, right, type_id, span, order);
            true
        }
        (_, KnownShape::Concrete { type_id, span }) => {
            let order = state.shape_authored_order(right);
            register_ordered_shape_witness(state, environment, left, type_id, span, order);
            true
        }
        _ => true,
    }
}

/// Union two declared substitutions of the same declared type, pairing edges by parameter.
///
/// WHAT: each left edge unions with the right edge for the same parameter identity. Every pair is
/// visited, so one refused pair never skips the rest.
/// WHY: canonical parameter order lets matching ranges merge linearly without positional pairing.
fn union_declared_bindings(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeBindingRange,
    right: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    let mut joined = true;
    let mut left_indices = left.indices();
    let mut right_indices = right.indices();
    let mut left_index = left_indices.next();
    let mut right_index = right_indices.next();
    while let (Some(left_current), Some(right_current)) = (left_index, right_index) {
        let edge = state.declared_binding(left_current);
        let peer = state.declared_binding(right_current);
        increment_ast_counter(AstCounter::NumericDeclaredBindingComparisons);
        match edge.parameter.0.cmp(&peer.parameter.0) {
            std::cmp::Ordering::Less => left_index = left_indices.next(),
            std::cmp::Ordering::Greater => right_index = right_indices.next(),
            std::cmp::Ordering::Equal => {
                memo.descent_started = true;
                joined &=
                    union_shape_classes_inner(state, environment, edge.shape, peer.shape, memo);
                left_index = left_indices.next();
                right_index = right_indices.next();
            }
        }
    }
    joined
}

/// Union two declared substitutions of different declared types through their projection.
///
/// WHAT: each side's declared type projects onto the other's, and the mapped edges union. A
/// projected structure pairs recursively with the opposing edges; a mismatch is left for coercion.
/// WHY: templates such as `Packet of A` and `Packet of B` agree only through their declared
/// mapping. Projecting both ways lets each side's concrete arguments reach the other side.
fn union_declared_envelopes(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left_declared: TypeId,
    left: KnownShapeBindingRange,
    right_declared: TypeId,
    right: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    let mut joined = union_projected_edges(
        state,
        environment,
        left_declared,
        left,
        right_declared,
        right,
        memo,
    );
    joined &= union_projected_edges(
        state,
        environment,
        right_declared,
        right,
        left_declared,
        left,
        memo,
    );
    joined
}

/// Pair one declared type's projection with the opposing edges and union the matched leaves.
///
/// WHAT: each `from` edge's projected pattern pairs with that edge through `union_pattern_shape`.
/// Identical declared types have no mapping to read, so they pair by parameter identity.
/// WHY: the projection is the declared mapping itself, so generic parameters relate only
/// symbolically and are never read as concrete values.
fn union_projected_edges(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    from_declared: TypeId,
    from: KnownShapeBindingRange,
    to_declared: TypeId,
    to: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    let key = (from_declared, from, to_declared, to);
    if let Some(result) = memo.projections.get(&key) {
        return *result;
    }
    let epoch = memo.epoch;
    let result = union_projected_edges_walk(
        state,
        environment,
        from_declared,
        from,
        to_declared,
        to,
        memo,
    );
    if memo.descent_started && memo.epoch == epoch {
        memo.projections.insert(key, result);
    }
    result
}

fn union_projected_edges_walk(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    from_declared: TypeId,
    from: KnownShapeBindingRange,
    to_declared: TypeId,
    to: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    if from_declared == to_declared {
        return union_declared_bindings(state, environment, from, to, memo);
    }
    let Some(projected) = declared_projection(environment, from_declared, to_declared) else {
        return true;
    };
    let mut joined = true;
    for index in from.indices() {
        let edge = state.declared_binding(index);
        let Some(projection) = projected.replacement_map().get(&edge.parameter).copied() else {
            continue;
        };
        memo.descent_started = true;
        joined &= union_pattern_shape(state, environment, projection, to, edge.shape, memo);
    }
    joined
}

/// Pair one declared type pattern with a shape and union the numeric leaves it names.
///
/// WHAT: `pattern` is a type in the parameter space of `peer`. A bare parameter unions the shape
/// with its peer edge. A fully concrete pattern witnesses the shape with its exact type. A
/// structured pattern recurses alongside a shape of the same kind: a collection pairs its element
/// only when fixed capacities match, map keys and values and optional inners pair one-to-one, and
/// a nested declared substitution pairs
/// through its own projection. A concrete shape witnesses the peer edges its pattern names. Any
/// other mismatch is left for coercion to report.
/// WHY: `Packet of A` projects onto `Packet of {B}` as `A -> {B}`, so the numeric leaf behind `B`
/// joins only when the pattern is walked against the edge's existing shape. That walk allocates no
/// synthetic envelope node per comparison.
fn union_pattern_shape(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    pattern: TypeId,
    peer: KnownShapeBindingRange,
    shape: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    let key = (pattern, peer, shape);
    if let Some(result) = memo.patterns.get(&key) {
        return *result;
    }
    let epoch = memo.epoch;
    let result = union_pattern_shape_walk(state, environment, pattern, peer, shape, memo);
    if memo.descent_started && memo.epoch == epoch {
        memo.patterns.insert(key, result);
    }
    result
}

fn union_pattern_shape_walk(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    pattern: TypeId,
    peer: KnownShapeBindingRange,
    shape: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    if let Some(parameter) = environment.bare_generic_parameter(pattern) {
        return match state.declared_binding_shape(peer, parameter) {
            Some(peer_shape) => {
                union_shape_classes_inner(state, environment, shape, peer_shape, memo)
            }
            None => true,
        };
    }
    if !environment.contains_generic_parameter(pattern) {
        let order = state.shape_authored_order(shape);
        register_ordered_shape_witness(state, environment, shape, pattern, None, order);
        return true;
    }
    match state.shape(shape) {
        KnownShape::Concrete { type_id, span } => {
            let order = state.shape_authored_order(shape);
            witness_pattern_parameters(state, environment, pattern, peer, type_id, span, order);
            true
        }
        KnownShape::Numeric(_) => true,
        KnownShape::Collection {
            element,
            fixed_capacity,
        } => match environment.collection_shape(pattern) {
            Some(collection) if collection.fixed_capacity == fixed_capacity => {
                memo.descent_started = true;
                union_pattern_shape(
                    state,
                    environment,
                    collection.element_type,
                    peer,
                    element,
                    memo,
                )
            }
            _ => true,
        },
        KnownShape::Map { key, value } => match environment.map_shape(pattern) {
            Some(map) => {
                memo.descent_started = true;
                let keys = union_pattern_shape(state, environment, map.key_type, peer, key, memo);
                let values =
                    union_pattern_shape(state, environment, map.value_type, peer, value, memo);
                keys && values
            }
            None => true,
        },
        KnownShape::Optional { inner } => match environment.option_inner_type(pattern) {
            Some(inner_pattern) => {
                memo.descent_started = true;
                union_pattern_shape(state, environment, inner_pattern, peer, inner, memo)
            }
            None => true,
        },
        KnownShape::DeclaredSubstitution {
            declared_type_id,
            bindings,
        } => union_projected_edges(
            state,
            environment,
            declared_type_id,
            bindings,
            pattern,
            peer,
            memo,
        ),
    }
}

/// Witness the peer edges that a concrete shape's exact type binds through a pattern.
///
/// WHAT: matches the pattern against the concrete type, then gives each peer edge its bound type
/// through the natural witness rules.
/// WHY: a concrete subtree is an exact source for the leaves it reaches, as a concrete leaf is in
/// `union_shape_classes`. The existing type matcher performs the walk, so nothing is staged.
fn witness_pattern_parameters(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    pattern: TypeId,
    peer: KnownShapeBindingRange,
    concrete: TypeId,
    span: Option<SourceSpan>,
    authored_order: u64,
) {
    let Some(matched) = declared_projection(environment, pattern, concrete) else {
        return;
    };
    let mut visited = FxHashSet::default();
    for index in peer.indices() {
        let edge = state.declared_binding(index);
        if let Some(bound) = matched.replacement_map().get(&edge.parameter).copied() {
            register_ordered_shape_witness_inner(
                state,
                environment,
                edge.shape,
                bound,
                span,
                authored_order,
                &mut visited,
            );
        }
    }
}

/// Match one declared type against another and return the projection of the first's parameters.
///
/// WHAT: `None` when the declared types do not match or the match conflicts.
/// WHY: a mismatch is not reported here; ordinary coercion reports it against the canonical types.
pub(crate) fn declared_projection(
    environment: &TypeEnvironment,
    from_declared: TypeId,
    to_declared: TypeId,
) -> Option<GenericTypeBindings> {
    let mut projected = GenericTypeBindings::new();
    match environment.try_collect_type_parameter_bindings_typeid(
        from_declared,
        to_declared,
        &mut projected,
    ) {
        Ok(true) => Some(projected),
        _ => None,
    }
}

/// Whether two numeric classes may not share one equality class.
///
/// WHAT: an integer-pinned origin (a bound whole or a loop item) refuses a merged decimal family
/// on either side. A bound binary float keeps its decimal family, so it still joins decimal and
/// mixed peers.
/// WHY: an integer-pinned origin keeps its integer family, so a decimal peer must never reclassify
/// it.
pub(crate) fn numeric_classes_join_refused(
    state: &mut NumericConstructionState,
    left: NumericClassId,
    right: NumericClassId,
) -> bool {
    (state.class_pinned_family(left) == Some(NumericFamily::Whole)
        && state.class_family_is_decimal(right))
        || (state.class_pinned_family(right) == Some(NumericFamily::Whole)
            && state.class_family_is_decimal(left))
}

/// Whether joining two shapes would retype an integer-pinned origin with a decimal peer.
///
/// WHAT: refuses a numeric/numeric pair when one side is an integer-pinned origin (a bound whole
///       or a loop item) and the other's merged family is decimal. Recurses through collection
///       elements, map keys and values, optional inners and declared substitution edges. Concrete
///       leaves and mismatched structure never refuse here; coercion reports them.
/// WHY: an integer-pinned origin keeps its integer family at every nesting level, so the join is
///      refused before any union. The refused fragment then completes against its side's own type.
pub(crate) fn shapes_join_refused(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeId,
    right: KnownShapeId,
) -> bool {
    let mut memo = ShapePairMemo::default();
    shapes_join_refused_inner(state, environment, left, right, &mut memo)
}

fn shapes_join_refused_inner(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeId,
    right: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    match (state.shape(left), state.shape(right)) {
        (KnownShape::Numeric(left_class), KnownShape::Numeric(right_class)) => {
            return numeric_classes_join_refused(state, left_class, right_class);
        }
        (KnownShape::Collection { .. }, KnownShape::Collection { .. })
        | (KnownShape::Map { .. }, KnownShape::Map { .. })
        | (KnownShape::Optional { .. }, KnownShape::Optional { .. })
        | (KnownShape::DeclaredSubstitution { .. }, KnownShape::DeclaredSubstitution { .. }) => {}
        _ => return false,
    }
    let key = (left, right);
    if let Some(result) = memo.pairs.get(&key) {
        return *result;
    }
    let epoch = memo.epoch;
    let result = shapes_join_refused_walk(state, environment, left, right, memo);
    if memo.descent_started && memo.epoch == epoch {
        memo.pairs.insert(key, result);
    }
    result
}

fn shapes_join_refused_walk(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeId,
    right: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    match (state.shape(left), state.shape(right)) {
        (KnownShape::Numeric(left_class), KnownShape::Numeric(right_class)) => {
            numeric_classes_join_refused(state, left_class, right_class)
        }
        (
            KnownShape::Collection {
                element: left_element,
                ..
            },
            KnownShape::Collection {
                element: right_element,
                ..
            },
        ) => {
            memo.descent_started = true;
            shapes_join_refused_inner(state, environment, left_element, right_element, memo)
        }
        (
            KnownShape::Map {
                key: left_key,
                value: left_value,
            },
            KnownShape::Map {
                key: right_key,
                value: right_value,
            },
        ) => {
            memo.descent_started = true;
            shapes_join_refused_inner(state, environment, left_key, right_key, memo)
                || shapes_join_refused_inner(state, environment, left_value, right_value, memo)
        }
        (
            KnownShape::Optional { inner: left_inner },
            KnownShape::Optional { inner: right_inner },
        ) => {
            memo.descent_started = true;
            shapes_join_refused_inner(state, environment, left_inner, right_inner, memo)
        }
        (
            KnownShape::DeclaredSubstitution {
                declared_type_id: left_declared,
                bindings: left_bindings,
            },
            KnownShape::DeclaredSubstitution {
                declared_type_id: right_declared,
                bindings: right_bindings,
            },
        ) => {
            if left_declared == right_declared {
                declared_bindings_refused(state, environment, left_bindings, right_bindings, memo)
            } else {
                declared_envelopes_refused(
                    state,
                    environment,
                    left_declared,
                    left_bindings,
                    right_declared,
                    right_bindings,
                    memo,
                )
            }
        }
        _ => false,
    }
}

/// Whether any identically declared edge pair would refuse the join.
fn declared_bindings_refused(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left: KnownShapeBindingRange,
    right: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    let mut refused = false;
    let mut left_indices = left.indices();
    let mut right_indices = right.indices();
    let mut left_index = left_indices.next();
    let mut right_index = right_indices.next();
    while let (Some(left_current), Some(right_current)) = (left_index, right_index) {
        let edge = state.declared_binding(left_current);
        let peer = state.declared_binding(right_current);
        increment_ast_counter(AstCounter::NumericDeclaredBindingComparisons);
        match edge.parameter.0.cmp(&peer.parameter.0) {
            std::cmp::Ordering::Less => left_index = left_indices.next(),
            std::cmp::Ordering::Greater => right_index = right_indices.next(),
            std::cmp::Ordering::Equal => {
                memo.descent_started = true;
                refused |=
                    shapes_join_refused_inner(state, environment, edge.shape, peer.shape, memo);
                left_index = left_indices.next();
                right_index = right_indices.next();
            }
        }
    }
    refused
}

/// Whether a projection of a different declared type refuses any projected edge pair.
fn declared_envelopes_refused(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    left_declared: TypeId,
    left: KnownShapeBindingRange,
    right_declared: TypeId,
    right: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    let mut refused = projected_edges_refused(
        state,
        environment,
        left_declared,
        left,
        right_declared,
        right,
        memo,
    );
    refused |= projected_edges_refused(
        state,
        environment,
        right_declared,
        right,
        left_declared,
        left,
        memo,
    );
    refused
}

/// Whether the edges of one projection refuse their peers through the join's own pairing.
///
/// WHAT: each `from` edge's projected pattern is checked by `pattern_shapes_refused`. Identical
/// declared types pair by parameter identity, as `union_projected_edges` does.
/// WHY: refusal must follow the same pairs the join unions, or a nested bound whole could merge
/// with a decimal peer that the join never reached.
fn projected_edges_refused(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    from_declared: TypeId,
    from: KnownShapeBindingRange,
    to_declared: TypeId,
    to: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    let key = (from_declared, from, to_declared, to);
    if let Some(result) = memo.projections.get(&key) {
        return *result;
    }
    let epoch = memo.epoch;
    let result = projected_edges_refused_walk(
        state,
        environment,
        from_declared,
        from,
        to_declared,
        to,
        memo,
    );
    if memo.descent_started && memo.epoch == epoch {
        memo.projections.insert(key, result);
    }
    result
}

fn projected_edges_refused_walk(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    from_declared: TypeId,
    from: KnownShapeBindingRange,
    to_declared: TypeId,
    to: KnownShapeBindingRange,
    memo: &mut ShapePairMemo,
) -> bool {
    if from_declared == to_declared {
        return declared_bindings_refused(state, environment, from, to, memo);
    }
    let Some(projected) = declared_projection(environment, from_declared, to_declared) else {
        return false;
    };
    let mut refused = false;
    for index in from.indices() {
        let edge = state.declared_binding(index);
        let Some(projection) = projected.replacement_map().get(&edge.parameter).copied() else {
            continue;
        };
        memo.descent_started = true;
        refused |= pattern_shapes_refused(state, environment, projection, to, edge.shape, memo);
    }
    refused
}

/// Whether a declared type pattern paired with a shape refuses any numeric pair.
///
/// WHAT: mirrors `union_pattern_shape`. A bare parameter checks its peer edge, and structured
/// patterns recurse along the same shape kinds. Concrete shapes and mismatched structure never
/// refuse here.
/// WHY: a bound whole nested inside a projected pattern must refuse a decimal peer exactly where
/// the join would have paired them.
fn pattern_shapes_refused(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    pattern: TypeId,
    peer: KnownShapeBindingRange,
    shape: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    let key = (pattern, peer, shape);
    if let Some(result) = memo.patterns.get(&key) {
        return *result;
    }
    let epoch = memo.epoch;
    let result = pattern_shapes_refused_walk(state, environment, pattern, peer, shape, memo);
    if memo.descent_started && memo.epoch == epoch {
        memo.patterns.insert(key, result);
    }
    result
}

fn pattern_shapes_refused_walk(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    pattern: TypeId,
    peer: KnownShapeBindingRange,
    shape: KnownShapeId,
    memo: &mut ShapePairMemo,
) -> bool {
    if let Some(parameter) = environment.bare_generic_parameter(pattern) {
        return match state.declared_binding_shape(peer, parameter) {
            Some(peer_shape) => {
                shapes_join_refused_inner(state, environment, shape, peer_shape, memo)
            }
            None => false,
        };
    }
    if !environment.contains_generic_parameter(pattern) {
        return false;
    }
    match state.shape(shape) {
        KnownShape::Concrete { .. } | KnownShape::Numeric(_) => false,
        KnownShape::Collection {
            element,
            fixed_capacity,
        } => match environment.collection_shape(pattern) {
            Some(collection) if collection.fixed_capacity == fixed_capacity => {
                memo.descent_started = true;
                pattern_shapes_refused(
                    state,
                    environment,
                    collection.element_type,
                    peer,
                    element,
                    memo,
                )
            }
            _ => false,
        },
        KnownShape::Map { key, value } => match environment.map_shape(pattern) {
            Some(map) => {
                memo.descent_started = true;
                pattern_shapes_refused(state, environment, map.key_type, peer, key, memo)
                    || pattern_shapes_refused(state, environment, map.value_type, peer, value, memo)
            }
            None => false,
        },
        KnownShape::Optional { inner } => match environment.option_inner_type(pattern) {
            Some(inner_pattern) => {
                memo.descent_started = true;
                pattern_shapes_refused(state, environment, inner_pattern, peer, inner, memo)
            }
            None => false,
        },
        KnownShape::DeclaredSubstitution {
            declared_type_id,
            bindings,
        } => projected_edges_refused(
            state,
            environment,
            declared_type_id,
            bindings,
            pattern,
            peer,
            memo,
        ),
    }
}

/// Pin every numeric origin reachable from a shape at the given pin level.
///
/// WHAT: each numeric leaf takes the pin when `NumericConstructionState::pin_numeric_origin`
///       admits it for its merged family; concrete leaves are untouched.
/// WHY: a bound or loop-item origin keeps its family at every nesting level, so a later peer its
///      family rejects must be refused rather than retype the origin. Fresh dense marks visit shared
///      predecessors once per walk without hiding a stronger pin requested by a later declaration.
pub(crate) fn pin_numeric_shape(
    state: &mut NumericConstructionState,
    shape: KnownShapeId,
    pin: NumericPin,
) {
    match state.shape(shape) {
        KnownShape::Concrete { .. } => {
            increment_ast_counter(AstCounter::NumericShapePinNodesVisited);
            return;
        }
        KnownShape::Numeric(class) => {
            increment_ast_counter(AstCounter::NumericShapePinNodesVisited);
            state.pin_numeric_origin(class, pin);
            return;
        }
        _ => {}
    }
    let epoch = state.begin_shape_pin_walk();
    pin_numeric_shape_inner(state, shape, pin, epoch);
}

fn pin_numeric_shape_inner(
    state: &mut NumericConstructionState,
    shape: KnownShapeId,
    pin: NumericPin,
    epoch: u32,
) {
    if !state.visit_shape_pin_node(shape, epoch) {
        return;
    }
    increment_ast_counter(AstCounter::NumericShapePinNodesVisited);
    match state.shape(shape) {
        KnownShape::Concrete { .. } => {}
        KnownShape::Numeric(class) => state.pin_numeric_origin(class, pin),
        KnownShape::Collection { element, .. } => {
            pin_numeric_shape_inner(state, element, pin, epoch)
        }
        KnownShape::Map { key, value } => {
            pin_numeric_shape_inner(state, key, pin, epoch);
            pin_numeric_shape_inner(state, value, pin, epoch);
        }
        KnownShape::Optional { inner } => pin_numeric_shape_inner(state, inner, pin, epoch),
        KnownShape::DeclaredSubstitution { bindings, .. } => {
            for index in bindings.indices() {
                let edge = state.declared_binding(index);
                pin_numeric_shape_inner(state, edge.shape, pin, epoch);
            }
        }
    }
}

/// Whether a shape holds any numeric leaf created for a named forward fragment.
///
/// WHAT: walks the shape structure; a forward leaf is one whose class carries the forward flag.
/// WHY: a forward entry completes from its natural result before any literal default is chosen.
pub(crate) fn shape_has_forward_leaf(
    state: &NumericConstructionState,
    shape: KnownShapeId,
) -> bool {
    fn visit(
        state: &NumericConstructionState,
        shape: KnownShapeId,
        visited: &mut FxHashSet<KnownShapeId>,
    ) -> bool {
        if !visited.insert(shape) {
            return false;
        }
        match state.shape(shape) {
            KnownShape::Concrete { .. } => false,
            KnownShape::Numeric(class) => state.class_is_forward(class),
            KnownShape::Collection { element, .. } => visit(state, element, visited),
            KnownShape::Map { key, value } => {
                visit(state, key, visited) || visit(state, value, visited)
            }
            KnownShape::Optional { inner } => visit(state, inner, visited),
            KnownShape::DeclaredSubstitution { bindings, .. } => bindings
                .indices()
                .any(|index| visit(state, state.declared_binding(index).shape, visited)),
        }
    }

    match state.shape(shape) {
        KnownShape::Concrete { .. } => false,
        KnownShape::Numeric(class) => state.class_is_forward(class),
        _ => visit(state, shape, &mut FxHashSet::default()),
    }
}

/// The canonical type a numeric class resolves to at completion.
///
/// WHAT: the merged exact receiver type when witnesses exist, otherwise the class family default.
/// WHY: defaults are applied only here, after every leaf constraint has been collected.
pub(crate) fn numeric_class_type(
    state: &mut NumericConstructionState,
    class: NumericClassId,
) -> ConstructionResult<TypeId> {
    match merged_receiver_type(state, class)? {
        Some(type_id) => Ok(type_id),
        None => Ok(state.class_family(class).default_type_id()),
    }
}

/// Materialise a known shape into its canonical type after all constraints are collected.
///
/// WHAT: `Concrete` returns its exact type; `Numeric` resolves its class leaf type; `Collection`
/// and `Map` intern through the existing environment helpers, preserving fixed capacity.
/// WHY: canonical `TypeId`s are created only once every numeric leaf is concrete. This function
/// never invents a fallback type for an unresolved shape.
pub(crate) fn materialise_known_shape(
    state: &mut NumericConstructionState,
    environment: &mut TypeEnvironment,
    shape: KnownShapeId,
) -> ConstructionResult<TypeId> {
    fn visit(
        state: &mut NumericConstructionState,
        environment: &mut TypeEnvironment,
        shape: KnownShapeId,
        memo: &mut FxHashMap<KnownShapeId, TypeId>,
    ) -> ConstructionResult<TypeId> {
        if let Some(type_id) = memo.get(&shape) {
            return Ok(*type_id);
        }
        let type_id = match state.shape(shape) {
            KnownShape::Concrete { type_id, .. } => type_id,
            KnownShape::Numeric(class) => numeric_class_type(state, class)?,
            KnownShape::Collection {
                element,
                fixed_capacity,
            } => {
                let element_type = visit(state, environment, element, memo)?;
                environment.intern_collection(element_type, fixed_capacity)
            }
            KnownShape::Map { key, value } => {
                let key_type = visit(state, environment, key, memo)?;
                let value_type = visit(state, environment, value, memo)?;
                environment.intern_map(key_type, value_type)
            }
            KnownShape::Optional { inner } => {
                let inner_type = visit(state, environment, inner, memo)?;
                environment.intern_option(inner_type)
            }
            KnownShape::DeclaredSubstitution {
                declared_type_id,
                bindings,
            } => {
                let mut mapping = FxHashMap::default();
                for index in bindings.indices() {
                    let edge = state.declared_binding(index);
                    let type_id = visit(state, environment, edge.shape, memo)?;
                    mapping.insert(edge.parameter, type_id);
                }
                let materialised = environment.substitute_type_id(declared_type_id, &mapping);
                if environment.contains_generic_parameter(materialised) {
                    return Err(CompilerError::compiler_error(
                        "declared substitution is missing a binding for a parameter its type mentions",
                    )
                    .into());
                }
                materialised
            }
        };
        memo.insert(shape, type_id);
        Ok(type_id)
    }

    match state.shape(shape) {
        KnownShape::Concrete { type_id, .. } => Ok(type_id),
        KnownShape::Numeric(class) => numeric_class_type(state, class),
        // Resolution can change between invocations, so only this traversal owns the memo.
        _ => visit(state, environment, shape, &mut FxHashMap::default()),
    }
}

/// Project every exact receiver of a shape onto its numeric leaves, without defaulting any leaf.
///
/// WHAT: a numeric leaf whose class has a merged exact receiver becomes that concrete type, carrying
///       the span of its first receiver. A leaf with no receiver stays numeric, and a node is rebuilt
///       only when one of its children changed.
/// WHY: every result envelope of one generic origin shares the exact width and precision of the
///      receiver that fixed it. A partly solved envelope keeps its solved leaves exact and leaves the
///      unsolved ones pending. Never defaulting here means an unsolved class is never taken for a
///      known type.
pub(crate) fn project_exact_shape(
    state: &mut NumericConstructionState,
    shape: KnownShapeId,
) -> ConstructionResult<KnownShapeId> {
    fn project_leaf(
        state: &mut NumericConstructionState,
        shape: KnownShapeId,
        class: NumericClassId,
    ) -> ConstructionResult<KnownShapeId> {
        let Some(type_id) = merged_receiver_type(state, class)? else {
            return Ok(shape);
        };
        let witness = state
            .first_receiver_witness(class)
            .expect("a merged exact type has a receiving witness");
        state.intern_ordered_shape(
            KnownShape::Concrete {
                type_id,
                span: witness.span,
            },
            witness.authored_order,
        )
    }

    fn visit(
        state: &mut NumericConstructionState,
        shape: KnownShapeId,
        memo: &mut FxHashMap<KnownShapeId, KnownShapeId>,
    ) -> ConstructionResult<KnownShapeId> {
        if let Some(projected) = memo.get(&shape) {
            return Ok(*projected);
        }
        let projected = match state.shape(shape) {
            KnownShape::Concrete { .. } => shape,
            KnownShape::Numeric(class) => project_leaf(state, shape, class)?,
            KnownShape::Collection {
                element,
                fixed_capacity,
            } => {
                let projected = visit(state, element, memo)?;
                if projected == element {
                    shape
                } else {
                    let order = state.shape_authored_order(shape);
                    state.intern_ordered_shape(
                        KnownShape::Collection {
                            element: projected,
                            fixed_capacity,
                        },
                        order,
                    )?
                }
            }
            KnownShape::Map { key, value } => {
                let projected_key = visit(state, key, memo)?;
                let projected_value = visit(state, value, memo)?;
                if projected_key == key && projected_value == value {
                    shape
                } else {
                    let order = state.shape_authored_order(shape);
                    state.intern_ordered_shape(
                        KnownShape::Map {
                            key: projected_key,
                            value: projected_value,
                        },
                        order,
                    )?
                }
            }
            KnownShape::Optional { inner } => {
                let projected = visit(state, inner, memo)?;
                if projected == inner {
                    shape
                } else {
                    let order = state.shape_authored_order(shape);
                    state.intern_ordered_shape(KnownShape::Optional { inner: projected }, order)?
                }
            }
            KnownShape::DeclaredSubstitution {
                declared_type_id,
                bindings,
            } => {
                let mut edges = Vec::new();
                for index in bindings.indices() {
                    let edge = state.declared_binding(index);
                    let projected = visit(state, edge.shape, memo)?;
                    if projected != edge.shape && edges.is_empty() {
                        // Unchanged envelopes need no scratch allocation. Once a leaf changes,
                        // retain the unchanged prefix and projected suffix for one replacement.
                        edges.reserve(bindings.indices().len());
                        for previous in bindings.indices().take_while(|previous| *previous != index)
                        {
                            edges.push(state.declared_binding(previous));
                        }
                    }
                    if !edges.is_empty() || projected != edge.shape {
                        edges.push(KnownShapeBinding {
                            parameter: edge.parameter,
                            shape: projected,
                        });
                    }
                }
                if edges.is_empty() {
                    shape
                } else {
                    state.intern_declared_substitution(declared_type_id, &edges)?
                }
            }
        };
        // Unchanged identities also need memoising, otherwise open shared DAGs re-expand.
        memo.insert(shape, projected);
        Ok(projected)
    }

    match state.shape(shape) {
        KnownShape::Concrete { .. } => Ok(shape),
        KnownShape::Numeric(class) => project_leaf(state, shape, class),
        _ => visit(state, shape, &mut FxHashMap::default()),
    }
}

/// Whether a shape still holds a numeric leaf with no merged receiver.
///
/// WHY: an open leaf is the only reason an exact projection cannot materialise, so the test is
///      structural and never consults a family default.
fn shape_has_open_leaf(state: &NumericConstructionState, shape: KnownShapeId) -> bool {
    fn visit(
        state: &NumericConstructionState,
        shape: KnownShapeId,
        visited: &mut FxHashSet<KnownShapeId>,
    ) -> bool {
        if !visited.insert(shape) {
            return false;
        }
        match state.shape(shape) {
            KnownShape::Concrete { .. } => false,
            KnownShape::Numeric(_) => true,
            KnownShape::Collection { element, .. } => visit(state, element, visited),
            KnownShape::Map { key, value } => {
                visit(state, key, visited) || visit(state, value, visited)
            }
            KnownShape::Optional { inner } => visit(state, inner, visited),
            KnownShape::DeclaredSubstitution { bindings, .. } => bindings
                .indices()
                .any(|index| visit(state, state.declared_binding(index).shape, visited)),
        }
    }

    match state.shape(shape) {
        KnownShape::Concrete { .. } => false,
        KnownShape::Numeric(_) => true,
        _ => visit(state, shape, &mut FxHashSet::default()),
    }
}

/// The exact canonical type of a shape, when every numeric leaf has a merged receiver.
///
/// WHAT: projects the receivers, then materialises the shape only when no numeric leaf stays open.
///       An open shape answers `None`.
/// WHY: an unannotated generic result takes a type only when that type is fully determined. Open
///      leaves are rejected before materialisation, so no family default is chosen here.
pub(crate) fn exact_shape_type(
    state: &mut NumericConstructionState,
    environment: &mut TypeEnvironment,
    shape: KnownShapeId,
) -> ConstructionResult<Option<TypeId>> {
    let projected = project_exact_shape(state, shape)?;
    if shape_has_open_leaf(state, projected) {
        return Ok(None);
    }
    materialise_known_shape(state, environment, projected).map(Some)
}

/// Pin every numeric leaf of a known shape to its current canonical type.
///
/// WHAT: adds one exact receiving witness per numeric class, carrying the type that class resolves
///       to right now (merged receiver, else family default).
/// WHY: a cyclic use seeds a binding's storage from these facts before its recipe finishes. The
///      pinned witnesses make any later conflicting witness fail, so the storage type cannot change
///      silently when the deferred producer completes. The caller supplies the declaration's
///      parse-time key because these completion-time witnesses label that declaration, not the
///      later moment its storage was seeded.
pub(crate) fn pin_numeric_leaves(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    shape: KnownShapeId,
    span: Option<SourceSpan>,
    authored_order: u64,
) -> ConstructionResult<()> {
    fn visit(
        state: &mut NumericConstructionState,
        environment: &TypeEnvironment,
        shape: KnownShapeId,
        span: Option<SourceSpan>,
        authored_order: u64,
        visited: &mut FxHashSet<KnownShapeId>,
    ) -> ConstructionResult<()> {
        if !visited.insert(shape) {
            return Ok(());
        }
        match state.shape(shape) {
            KnownShape::Concrete { .. } => Ok(()),
            KnownShape::Numeric(class) => {
                let type_id = numeric_class_type(state, class)?;
                state.add_ordered_receiver_witness(
                    class,
                    type_id,
                    span,
                    authored_order,
                    environment,
                );
                Ok(())
            }
            KnownShape::Collection { element, .. } => {
                visit(state, environment, element, span, authored_order, visited)
            }
            KnownShape::Map { key, value } => {
                visit(state, environment, key, span, authored_order, visited)?;
                visit(state, environment, value, span, authored_order, visited)
            }
            KnownShape::Optional { inner } => {
                visit(state, environment, inner, span, authored_order, visited)
            }
            KnownShape::DeclaredSubstitution { bindings, .. } => {
                for index in bindings.indices() {
                    let edge = state.declared_binding(index);
                    visit(
                        state,
                        environment,
                        edge.shape,
                        span,
                        authored_order,
                        visited,
                    )?;
                }
                Ok(())
            }
        }
    }

    match state.shape(shape) {
        KnownShape::Concrete { .. } => Ok(()),
        KnownShape::Numeric(class) => {
            let type_id = numeric_class_type(state, class)?;
            state.add_ordered_receiver_witness(class, type_id, span, authored_order, environment);
            Ok(())
        }
        _ => visit(
            state,
            environment,
            shape,
            span,
            authored_order,
            &mut FxHashSet::default(),
        ),
    }
}

/// The known shape a directed recipe's value has, decided once when its binding registers.
///
/// WHAT: a fragment that is exactly one retained literal, or exactly one retained operand whose
/// known result is a shape (a collection element, field, method, builtin or catch-protected value),
/// answers that shape. Named arithmetic and every other multi-item fragment answers `None`, as does
/// an operand with no known shape or a zero-result operand.
/// WHY: the recipe is the sole owner of a directed value's shape. Deciding it here from the
/// authored fragment keeps eligible element and capacity relations without minting a numeric class
/// for named arithmetic, which stays forward-only.
pub(crate) fn directed_recipe_shape(
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    items: &[ExpressionRpnItem],
) -> ConstructionResult<Option<KnownShapeId>> {
    if let Some(shape) = retained_known_shape(items) {
        return Ok(Some(shape));
    }
    match items {
        [item] => retained_operand_result_shape(item, scope_context, environment, false),
        _ => Ok(None),
    }
}

/// The construction shape of one retained numeric fragment used as a literal element, key or value.
///
/// WHAT: a retained nested literal keeps its own shape; a final cast keeps its exact target type;
/// a single operand keeps the known result of its call, field, method or builtin through the
/// retained owner; a direct pending binding shares its binding's shape or class and pins a bound
/// origin; a comparison resolves to `Bool`. A literal-only fragment opens one numeric class
/// seeded with its authored family, or resolves real whole division to `F64`. Named arithmetic
/// opens one forward class with no authored family: its natural result is registered by
/// completion, never guessed from operands here.
/// WHY: a fragment's leaf constraints must reach its peers before any default applies, while a
/// non-numeric or operator-fixed result already has its exact type and must never become a numeric
/// class. A forward class never carries a family guess, so it cannot default an unrelated slot.
/// Callers must not hold a numeric state borrow: this function takes and releases it.
pub(crate) fn fragment_shape(
    scope_context: &ScopeContext,
    environment: &mut TypeEnvironment,
    items: &[ExpressionRpnItem],
) -> ConstructionResult<KnownShapeId> {
    if let Some(shape) = retained_known_shape(items) {
        return Ok(shape);
    }

    if let [
        ExpressionRpnItem::PendingNumericCast {
            target_type_id,
            requires_optional_wrap_after_cast,
            ..
        },
    ] = items
    {
        let type_id = if *requires_optional_wrap_after_cast {
            environment.intern_option(*target_type_id)
        } else {
            *target_type_id
        };
        return scope_context
            .numeric_state_mut()
            .intern_shape(KnownShape::Concrete {
                type_id,
                span: None,
            });
    }

    // The retained owner derives a single operand's known result. It reads the numeric state
    // itself, so it runs before any mutable borrow below.
    let operand_shape = match items {
        [item] => retained_operand_result_shape(item, scope_context, environment, false)?,
        _ => None,
    };
    if let Some(shape) = operand_shape {
        return Ok(shape);
    }

    if let Some(reference) = single_direct_pending_reference(items) {
        let mut state = scope_context.numeric_state_mut();
        if let Some(shape) = binding_reference_shape(&state, reference.binding) {
            pin_numeric_shape(&mut state, shape, NumericPin::Bound);
            return Ok(shape);
        }
        let class = binding_class(&state, reference.binding);
        state.pin_numeric_origin(class, NumericPin::Bound);
        return state.intern_shape(KnownShape::Numeric(class));
    }

    // A comparison resolves to Bool whatever its operands are, so it never opens a class.
    if raw_fragment_result_is_independently_fixed(items, NumericFamily::Decimal) {
        return scope_context
            .numeric_state_mut()
            .intern_shape(KnownShape::Concrete {
                type_id: builtin_type_ids::BOOL,
                span: None,
            });
    }

    let family = NumericFamily::of_items(items);
    if items.iter().all(item_is_literal_recipe_shape) {
        // Literal-only real whole division resolves through the integer leaf domain to F64.
        if raw_fragment_result_is_independently_fixed(items, family) {
            return scope_context
                .numeric_state_mut()
                .intern_shape(KnownShape::Concrete {
                    type_id: builtin_type_ids::fixed_scalar(FixedScalar::F64),
                    span: None,
                });
        }
        let mut state = scope_context.numeric_state_mut();
        let class = state.create_class()?;
        state.note_class_family(class, family);
        return state.intern_shape(KnownShape::Numeric(class));
    }

    // Named arithmetic is forward-only: its natural result comes from completion, so the class
    // carries no authored family that could default an unrelated slot.
    let mut state = scope_context.numeric_state_mut();
    let class = state.create_class()?;
    state.mark_forward(class);
    state.intern_shape(KnownShape::Numeric(class))
}

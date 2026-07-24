//! Reactivity helpers for the JS runtime.
//!
//! WHAT: reactive source tracking, source-level invalidation scheduling, and backend-owned
//! template-string values with dependency metadata.
//! WHY: the HTML-JS V1 backend needs stable reactive sources, whole-source dirty marking, and a
//! runtime representation for template strings that carries dependency information without
//! exposing closures or function values to user code.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits reactive source runtime helpers.
    ///
    /// WHAT: `__moth_reactive_binding` constructs a binding record tagged with a stable source id;
    /// `__moth_reactive_schedule` marks a source dirty and batches a flush; `__moth_reactive_flush`
    /// rerenders every mounted fragment that depends on a dirty source.
    /// WHY: reactive source tracking is an opt-in runtime subsystem. Emitting it only when reachable
    /// emitted code declares or reads reactive sources keeps non-reactive bundles small.
    pub(crate) fn emit_runtime_reactive_source_helpers(&mut self) {
        self.emit_reactive_binding_helper();
        self.emit_reactive_scheduler_helpers();
    }

    /// Emits template-string runtime helpers.
    ///
    /// WHAT: `__moth_template_string` creates a backend-owned value carrying a snapshot function and
    /// a dependency array; `__moth_template_snapshot` flattens it to a plain string;
    /// `__moth_template_dependencies` reads direct/transitive source ids;
    /// `__moth_template_render_nested` renders a nested template value inside another template.
    /// WHY: reactive templates keep language type `String` but need a runtime representation that
    /// preserves dependency metadata for Phase 7 mounting and rerendering.
    pub(crate) fn emit_runtime_template_string_helpers(&mut self) {
        self.emit_template_string_helpers();
    }

    /// Emits the DOM mount helper that turns a runtime fragment value into a live slot.
    ///
    /// WHAT: `__moth_mount_template_fragment(slot, fragment)` inserts plain strings exactly like the
    /// non-reactive bootstrap, but treats reactive template-string values as mounted fragments: it
    /// renders the initial HTML, registers the fragment against every dependency source, and gives
    /// the fragment a `render()` function that the scheduler calls during a dirty-source flush.
    /// WHY: Phase 7 needs a single slot-hydration path that handles both plain-string
    ///      fragments and reactive template objects without leaking backend details into HIR.
    pub(crate) fn emit_runtime_mount_helper(&mut self) {
        self.emit_mount_helper();
    }

    /// Emits `__moth_reactive_binding`, a reactive-aware binding constructor.
    ///
    /// WHAT: returns the same reference-record shape as `__moth_binding`, plus `__moth_source_id`.
    /// WHY: later writes through this binding can identify which source to dirty without a
    /// parallel cell API.
    fn emit_reactive_binding_helper(&mut self) {
        self.emit_line("function __moth_reactive_binding(sourceId, value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_ref: true,");
                em.emit_line("__moth_kind: \"binding\",");
                em.emit_line("__moth_mode: \"slot\",");
                em.emit_line("__moth_slot: { value },");
                em.emit_line("__moth_target: null,");
                em.emit_line("__moth_source_id: sourceId,");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the reactive scheduler state and helpers.
    ///
    /// WHAT: module-level source-to-fragment dependency map, dirty-source set, and a batched
    /// microtask flush. `__moth_reactive_schedule(sourceId)` records the dirty source; the flush
    /// walks every mounted fragment and rerenders those that depend on at least one dirty source.
    /// WHY: source-level invalidation in V1 must be batched so multiple writes in one turn cause
    /// only one rerender pass. Phase 7 will register actual DOM mounts; Phase 6 schedules no-op
    /// flushes when no mounts exist.
    fn emit_reactive_scheduler_helpers(&mut self) {
        self.emit_line("const __moth_reactive_source_to_fragments = new Map();");
        self.emit_line("const __moth_reactive_dirty_sources = new Set();");
        self.emit_line("let __moth_reactive_flush_scheduled = false;");
        self.emit_line("let __moth_reactive_flush_hook = null;");
        self.emit_line("");

        self.emit_line("function __moth_reactive_schedule(sourceId) {");
        self.with_indent(|emitter| {
            emitter.emit_line("__moth_reactive_dirty_sources.add(sourceId);");
            emitter.emit_line("if (__moth_reactive_flush_scheduled) {");
            emitter.with_indent(|em| em.emit_line("return;"));
            emitter.emit_line("}");
            emitter.emit_line("__moth_reactive_flush_scheduled = true;");
            emitter.emit_line("if (typeof queueMicrotask === \"function\") {");
            emitter.with_indent(|em| {
                em.emit_line("queueMicrotask(__moth_reactive_flush);");
            });
            emitter.emit_line("} else {");
            emitter.with_indent(|em| {
                em.emit_line("Promise.resolve().then(__moth_reactive_flush);");
            });
            emitter.emit_line("}");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_reactive_flush() {");
        self.with_indent(|emitter| {
            emitter.emit_line("__moth_reactive_flush_scheduled = false;");
            emitter.emit_line("if (__moth_reactive_dirty_sources.size === 0) {");
            emitter.with_indent(|em| em.emit_line("return;"));
            emitter.emit_line("}");
            emitter.emit_line("const dirty = Array.from(__moth_reactive_dirty_sources);");
            emitter.emit_line("__moth_reactive_dirty_sources.clear();");
            emitter.emit_line("for (const fragment of __moth_reactive_registered_fragments) {");
            emitter.with_indent(|em| {
                em.emit_line("if (fragment.dependencies.some(dep => dirty.includes(dep)) && fragment.render) {");
                em.with_indent(|inner| {
                    inner.emit_line("fragment.render();");
                });
                em.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (typeof __moth_reactive_flush_hook === \"function\") {");
            emitter.with_indent(|em| em.emit_line("__moth_reactive_flush_hook(dirty);"));
            emitter.emit_line("}");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("const __moth_reactive_registered_fragments = [];");
        self.emit_line("");
    }

    /// Emits template-string runtime helpers.
    ///
    /// WHAT: `__moth_template_string` creates a backend-owned value carrying a snapshot function and
    /// a dependency array; `__moth_template_snapshot` flattens it to a plain string;
    /// `__moth_template_dependencies` reads direct/transitive source ids;
    /// `__moth_template_render_nested` renders a nested template value inside another template.
    /// WHY: reactive templates keep language type `String` but need a runtime representation that
    /// preserves dependency metadata for Phase 7 mounting and rerendering.
    fn emit_template_string_helpers(&mut self) {
        self.emit_line("function __moth_template_string(snapshot, dependencies) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_template: true,");
                em.emit_line("snapshot,");
                em.emit_line("dependencies: dependencies || [],");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_template_snapshot(template) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (template === null || template === undefined) {");
            emitter.with_indent(|em| em.emit_line("return \"\";"));
            emitter.emit_line("}");
            emitter.emit_line("if (__moth_is_ref(template)) {");
            emitter.with_indent(|em| em.emit_line("return __moth_template_snapshot(__moth_read(template));"));
            emitter.emit_line("}");
            emitter.emit_line("if (typeof template === \"string\") {");
            emitter.with_indent(|em| em.emit_line("return template;"));
            emitter.emit_line("}");
            emitter.emit_line("if (template.__moth_template === true && typeof template.snapshot === \"function\") {");
            emitter.with_indent(|em| em.emit_line("return template.snapshot();"));
            emitter.emit_line("}");
            emitter.emit_line("return String(template);");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_template_dependencies(template) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (template === null || template === undefined || typeof template === \"string\") {");
            emitter.with_indent(|em| em.emit_line("return [];"));
            emitter.emit_line("}");
            emitter.emit_line("if (__moth_is_ref(template)) {");
            emitter.with_indent(|em| em.emit_line("return __moth_template_dependencies(__moth_read(template));"));
            emitter.emit_line("}");
            emitter.emit_line("if (template.__moth_template === true) {");
            emitter.with_indent(|em| em.emit_line("return template.dependencies || [];"));
            emitter.emit_line("}");
            emitter.emit_line("return [];");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_template_render_nested(template) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return __moth_template_snapshot(template);");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_template_collect_dependencies(directIds, nestedValues) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const result = [];");
            emitter.emit_line("if (directIds) {");
            emitter.with_indent(|em| {
                em.emit_line("for (const id of directIds) {");
                em.with_indent(|inner| inner.emit_line("result.push(id);"));
                em.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (nestedValues) {");
            emitter.with_indent(|em| {
                em.emit_line("for (const value of nestedValues) {");
                em.with_indent(|inner| {
                    inner.emit_line("const deps = __moth_template_dependencies(value);");
                    inner.emit_line("for (const dep of deps) {");
                    inner.with_indent(|deepest| deepest.emit_line("result.push(dep);"));
                    inner.emit_line("}");
                });
                em.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("return Array.from(new Set(result));");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits `__moth_mount_template_fragment`, the bridge between `start()` fragments and the DOM.
    ///
    /// WHAT: for a plain string fragment the helper behaves exactly like the direct bootstrap
    /// (`insertAdjacentHTML("beforeend", ...)`). For a reactive template-string value it creates a
    /// mounted fragment record, renders the initial HTML, registers the record against every
    /// dependency source, and exposes a `render()` closure that rerenders the whole slot.
    /// WHY: keeping this logic in one backend helper lets the HTML bootstrap stay a simple loop
    ///      while preserving source-order slot behavior and making rerendering a scheduler concern.
    fn emit_mount_helper(&mut self) {
        self.emit_line("function __moth_mount_template_fragment(slot, fragment) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!slot) {");
            emitter.with_indent(|em| em.emit_line("throw new Error(\"Missing runtime mount slot\");"));
            emitter.emit_line("}");
            emitter.emit_line("if (fragment === null || fragment === undefined || typeof fragment === \"string\") {");
            emitter.with_indent(|em| {
                em.emit_line("slot.insertAdjacentHTML(\"beforeend\", fragment || \"\");");
                em.emit_line("return;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (fragment.__moth_template === true && typeof fragment.snapshot === \"function\") {");
            emitter.with_indent(|mounted_emitter| {
                mounted_emitter.emit_line("var mounted = {");
                mounted_emitter.with_indent(|em| {
                    em.emit_line("slot: slot,");
                    em.emit_line("dependencies: fragment.dependencies || [],");
                    em.emit_line("render: function() {");
                    em.with_indent(|inner| {
                        inner.emit_line("mounted.slot.innerHTML = \"\";");
                        inner.emit_line("mounted.slot.insertAdjacentHTML(\"beforeend\", fragment.snapshot());");
                    });
                    em.emit_line("}");
                });
                mounted_emitter.emit_line("};");
                mounted_emitter.emit_line("mounted.slot.innerHTML = \"\";");
                mounted_emitter.emit_line("mounted.slot.insertAdjacentHTML(\"beforeend\", fragment.snapshot());");
                mounted_emitter.emit_line("for (var i = 0; i < mounted.dependencies.length; i++) {");
                mounted_emitter.with_indent(|em| {
                    em.emit_line("var sourceId = mounted.dependencies[i];");
                    em.emit_line("var list = __moth_reactive_source_to_fragments.get(sourceId);");
                    em.emit_line("if (!list) {");
                    em.with_indent(|inner| {
                        inner.emit_line("list = [];");
                        inner.emit_line("__moth_reactive_source_to_fragments.set(sourceId, list);");
                    });
                    em.emit_line("}");
                    em.emit_line("list.push(mounted);");
                });
                mounted_emitter.emit_line("}");
                mounted_emitter.emit_line("__moth_reactive_registered_fragments.push(mounted);");
                mounted_emitter.emit_line("return;");
            });
            emitter.emit_line("}");
            emitter.emit_line("slot.insertAdjacentHTML(\"beforeend\", String(fragment));");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}

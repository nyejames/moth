-- Fresh mutable parameters accept fresh rvalues (computed values, templates,
-- collection literals and struct constructors) without explicit `~`. Each
-- callee mutates its fresh local before returning an observable result, so one
-- primary owner covers the whole fresh-value contract under
-- `language.functions.mutable_parameter_accepts_fresh_values`.

Item = |
    label String,
|

mutate_computed_int |value ~Int| -> Int:
    value = value + 1
    return value
;

mutate_template_string |value ~String| -> String:
    value = "after"
    return value
;

mutate_collection |values ~{Int}| -> Int:
    ~values.push(4) catch:
    ;
    return values.length()
;

mutate_struct |value ~Item| -> String:
    value.label = "after"
    return value.label
;

render_fresh_values || -> String:
    computed_out = mutate_computed_int(1 + 2)
    template_out = mutate_template_string([:content])
    collection_out = mutate_collection({1, 2, 3})
    struct_out = mutate_struct(Item("x"))

    return [:function_call_mutable_param_fresh_values computed=[computed_out] template=[template_out] collection=[collection_out] struct=[struct_out]]
;

[:[render_fresh_values()]]

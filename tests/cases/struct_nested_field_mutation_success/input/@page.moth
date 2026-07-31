Inner = |
    value Int,
|

Outer = |
    inner Inner,
|

render_nested_mutation || -> String:
    wrapper ~= Outer(Inner(1))
    wrapper.inner.value = 5
    return [:struct_nested_field_mutation_success value=[wrapper.inner.value]]
;

[:[render_nested_mutation()]]

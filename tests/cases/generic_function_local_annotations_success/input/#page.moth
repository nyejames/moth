Box type T = |
    value T,
|

Maybe type T ::
    Some | value T |,
    Empty,
;

identity_copy type T |value T| -> T:
    stored T = value
    return stored
;

empty_values type T || -> {T}:
    values {T} = {}
    return values
;

wrap type T |value T| -> Box of T:
    item Box of T = Box(value)
    return item
;

choose type T |value T| -> Maybe of T:
    item Maybe of T = Maybe::Some(value)
    return item
;

value Int = identity_copy(41)
values {Int} = empty_values()
box Box of String = wrap("leaf")
choice Maybe of Int = choose(value)

io.line([: generic_function_local_annotations_success [values.length()]])

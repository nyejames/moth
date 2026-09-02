Box type T = |
    value T,
|

single type T |value T| -> {T}:
    return {value}
;

wrap_collection type T |value T| -> Box of {T}:
    return Box(single(value))
;

values {Int} = single(1)
second_values {Int} = single(2)
values_without_capacity {Int} = second_values
boxed_values Box of {String} = wrap_collection("leaf")

io.line([: generic_constructed_types_success [values.length()] [values_without_capacity.length()]])

empty_values type T || -> {T}:
    return {}
;

keep_values |values {Int}| -> {Int}:
    return values
;

inner_values {Int} = empty_values()
values {Int} = keep_values(inner_values)

io.line([: [values.length()]])

-- Validates that Int expressions can be contextually promoted to Float
-- when returned from a function with a Float return signature.

get_float || -> Float:
    return 1
;

get_float_expr || -> Float:
    return 2 + 3
;

result = get_float()
result2 = get_float_expr()

io.line("float returns from int passed")

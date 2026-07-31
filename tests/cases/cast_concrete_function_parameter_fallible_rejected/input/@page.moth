-- Plain `cast` with a fallible source in a concrete function parameter is rejected.

draw |x Int, y Int| -> Int:
    return x + y
;

value = draw(cast "1", cast "2")

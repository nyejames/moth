can_error || -> Int, Error!:
    return 1
;

accept |value Int| -> Int:
    return value
;

outer || -> Int:
    return accept(can_error())
;

io.line([: [outer()]])

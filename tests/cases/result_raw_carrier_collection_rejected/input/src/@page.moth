can_error || -> Int, Error!:
    return 1
;

outer || -> {Int}:
    return {can_error()}
;

io.line([: [outer()]])

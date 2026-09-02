can_error || -> Int, Error!:
    return 1
;

value = can_error() catch:
    then "fallback"
;

io.line([: [value]])

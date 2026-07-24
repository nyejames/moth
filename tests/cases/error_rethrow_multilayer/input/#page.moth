-- Error rethrow through multiple layers.

inner || -> String, Int, Error!:
    return! Error("origin")
;

middle || -> String, Int, Error!:
    v, n = inner()!
    return v, n
;

outer || -> String, Int, Error!:
    value, number = middle() catch |err|:
        return! err
    ;
    return value, number
;

result, count = outer() catch:

    then "outer_handler", 0

;
io.line([: [result]])
io.line([: [count]])

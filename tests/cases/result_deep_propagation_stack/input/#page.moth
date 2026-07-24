-- Four-layer propagation stack that unwinds through try/catch wrappers.
-- The innermost error should bubble all the way to the root fallback.

level_d || -> String, Error!:
    return! Error("origin")
;

level_c || -> String, Error!:
    return level_d()!
;

level_b || -> String, Error!:
    return level_c()!
;

level_a || -> String, Error!:
    return level_b()!
;

result = level_a() catch:

    then "deep_fallback"

;
io.line([: [result]])

pair_err |name String| -> String, Float, Error!:
    return! Error("boom")
;

recover || -> String, Float:
    first, score = pair_err("Ana") catch:
        then "x", 1.0
    ;
    return first, score
;

name, score = recover()
io.line([: [name]])
io.line([: [score]])

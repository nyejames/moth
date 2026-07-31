can_error |name String| -> String, Float, Error!:
    return! Error("boom")
;

recover |name String| -> String, Float:
    value, score = can_error(name) catch |err|:
        io.line([: [err.message]])
        then "guest", 0.0
    ;
    return value, score
;

name, score = recover("Ana")
io.line([: [name]])
io.line([: [score]])

fail || -> Error!:
    return! Error("boom")
;

recover ||:
    fail() catch |err|:
        io.line([: [err.message]])
        then "fallback"
    ;
;

recover()

fail || -> Error!:
    return! Error("boom")
;

recover || -> String:
    value = fail() catch:
        then "fallback"
    ;
    return value
;

io.line([: [recover()]])

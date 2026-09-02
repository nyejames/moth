load_user || -> String, Float, Error!:
    return! Error("missing")
;

recover || -> String, Float:
    name, score = load_user() catch:
        then "guest", 0.0
    ;
    return name, score
;

name, score = recover()
io.line("value catch multi bind")
io.line([: [name]])
io.line([: [score]])

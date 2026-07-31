can_error || -> Int, Error!:
    return! Error("bad")
;

recover || -> Int, Error!:
    value = can_error() catch |err| then return! err
    return value
;

io.line([: [recover()!]])

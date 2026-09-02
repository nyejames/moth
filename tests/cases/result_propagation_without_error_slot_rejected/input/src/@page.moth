can_error || -> String, Error!:
    return "ok"
;

recover || -> String:
    value = can_error()!
    return value
;

io.line([: [recover()]])

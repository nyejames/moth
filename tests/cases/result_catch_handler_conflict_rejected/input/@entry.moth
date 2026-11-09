can_error |value String| -> String, Error!:
    return value
;

recover |value String| -> String:
    err = "taken"
    output = can_error(value) catch |err|:
        io.line([: [err.message]])
        then "fallback"
    ;
    return output
;

io.line([: [recover("ok")]])

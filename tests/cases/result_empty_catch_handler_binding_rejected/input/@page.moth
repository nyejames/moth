can_error |value String| -> String, Error!:
    return value
;

recover |value String| -> String:
    return can_error(value) catch ||:
        then "fallback"
    ;
;

io.line([: [recover("ok")]])

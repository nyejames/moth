can_error |value String| -> String, Error!:
    return value
;

recover |value String| -> String:
    return can_error(value) catch |err, other|:
        then err.message
    ;
;

io.line([: [recover("ok")]])

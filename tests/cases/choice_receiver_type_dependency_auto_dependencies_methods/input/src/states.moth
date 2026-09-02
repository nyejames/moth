Status :: Ready, Busy;

label |this Status| -> String:
    result ~= "unknown"

    if this is:
        Ready => result = "ready"
        Busy => result = "busy"
    ;

    return result
;

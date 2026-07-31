Status :: Ready, Busy;

label |this Status| -> String:
    result ~= "unknown"

    if this is:
        Ready => result = "ready"
        Busy => result = "busy"
    ;

    return result
;

selected Status = Status::Busy
[:choice_receiver_method_basic_call value=[selected.label()]]

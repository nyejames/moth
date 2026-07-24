STATE_TEXT must:
    text |This| -> String
;

Status ::
    Ready,
    Busy,
;

text |this Status| -> String:
    result ~= "unknown"

    if this is:
        Ready => result = "ready"
        Busy => result = "busy"
    ;

    return result
;

Status must STATE_TEXT

selected Status = Status::Ready
[:trait_conformance_choice_success [selected.text()]]

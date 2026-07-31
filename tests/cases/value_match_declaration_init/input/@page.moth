Status ::
    Ready,
    Waiting,
;

label_for |status Status| -> String:
    label = if status is:
        Ready => then "ready"
        else => then "other"
    ;

    return label
;

label = label_for(Status::Ready)
io.line([:value match declaration init label=[label]])

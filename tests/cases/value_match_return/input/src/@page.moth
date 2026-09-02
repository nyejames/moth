Status ::
    Ready,
    Waiting,
;

label_for |status Status| -> String:
    return if status is:
        Ready => then "ready"
        else => then "other"
    ;
;

waiting = Status::Waiting
io.line([: [label_for(waiting)]])
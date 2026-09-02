-- Qualified choice arms in start body must not be misclassified as duplicate declarations.
Status ::
    Ready,
    Waiting,
;

current Status = Status::Ready

label = if current is:
    Status::Ready => then "ready"
    else => then "other"
;

io.line([:qualified arm label=[label]])

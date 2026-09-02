Status :: Ready, Busy;
current Status = Status::Ready
result ~= "unset"
if current is:
    Ready => result = "ready"
    Busy => result = "busy"
;
io.line([: [result]])

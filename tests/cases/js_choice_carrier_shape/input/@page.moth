Status :: Ready, Busy, Offline;
current Status = Status::Ready
result ~= "ok"
if current is:
    Ready => result = "ready"
    Busy => result = "busy"
    Offline => result = "offline"
;
io.line([: [result]])

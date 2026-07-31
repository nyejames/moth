can_error || -> Int, Error!:
    return! Error("bad")
;

recover || -> Int:
    return can_error() catch:
        then 0
        io.line("late")
    ;
;

io.line([: [recover()]])
io.line("then tail after value production is unreachable")

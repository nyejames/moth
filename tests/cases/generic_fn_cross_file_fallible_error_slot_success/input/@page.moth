import @./helpers {read_or_raise}

first = read_or_raise(7, Error("bad"), false) catch:
    then 0
;
second = read_or_raise(7, Error("bad"), true) catch:
    then -1
;
io.line([: [first] [second]])

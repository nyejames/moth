import @utils {exported_read_or_raise}

first = exported_read_or_raise(7, Error("bad"), false) catch:
    then 0
;
second = exported_read_or_raise(7, Error("bad"), true) catch:
    then -1
;
io.line([: [first] [second]])

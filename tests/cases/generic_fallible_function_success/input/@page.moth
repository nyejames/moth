read_or_raise type T, E |value T, err E, fail Bool| -> T, E!:
    if fail:
        return! err
    ;

    return value
;

first = read_or_raise(7, Error("bad"), false) catch:

    then 0

;
second = read_or_raise(7, Error("bad"), true) catch:
    then -1
;
io.line([: [first] [second]])

can_error || -> Error!:
    return! Error("bad")
;

value = if true:
    can_error() catch:
        then 1
    ;
else
    then 0
;

io.line([: [value]])

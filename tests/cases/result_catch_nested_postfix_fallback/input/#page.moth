inner || -> Int, Error!:
    return 1
;

outer || -> Int, Error!:
    return! Error("outer_boom")
;

recover || -> Int, Error!:
    return outer() catch:
        then inner()!
    ;
;

value = recover() catch:
    then 0
;

io.line([: [value]])

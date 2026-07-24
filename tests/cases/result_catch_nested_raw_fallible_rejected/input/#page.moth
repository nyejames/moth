inner || -> Int, Error!:
    return 1
;

outer || -> Int, Error!:
    return 2
;

recover || -> Int:
    return outer() catch:
        then inner()
    ;
;

io.line([: [recover()]])

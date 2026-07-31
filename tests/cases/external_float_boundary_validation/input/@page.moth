import @./bad_float.js {unsafe_float}

load_code || -> Int, Error!:
    value = unsafe_float()

    if value > 0.0:
        return 1
    ;

    return 0
;

code = load_code() catch |err|:
    then err.code
;

io.line([: external float boundary code=[code]])

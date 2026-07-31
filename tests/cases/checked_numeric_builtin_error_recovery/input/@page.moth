overflow_int || -> Int, Error!:
    x = 2147483647
    return x + 1
;

nonfinite_float || -> Float, Error!:
    x = 1e200
    return x * 1e200
;

int_code = overflow_int() catch |int_err|:
    then int_err.code
;

float_code = nonfinite_float() catch |float_err|:
    then float_err.code
;

io.line([: numeric recovery codes=[int_code]/[float_code]])

parse_string |text String| -> Int, Error!:
    return cast! text
;

parse_float |value Float| -> Int, Error!:
    return cast! value
;

too_big_string = parse_string("2147483648") catch |big_string_err|:
    then big_string_err.code
;

too_small_string = parse_string("-2147483649") catch |small_string_err|:
    then small_string_err.code
;

too_big_float = parse_float(2147483648.0) catch |big_float_err|:
    then big_float_err.code
;

too_small_float = parse_float(-2147483649.0) catch |small_float_err|:
    then small_float_err.code
;

[: runtime i32 boundary catch codes: [too_big_string]/[too_small_string]/[too_big_float]/[too_small_float]]

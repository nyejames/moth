parse_string |text String| -> Int, Error!:
    return cast! text
;

parse_float |value Float| -> Int, Error!:
    return cast! value
;

max_from_string = parse_string("2147483647") catch:
    then -1
;

min_from_string = parse_string("-2147483648") catch:
    then 1
;

max_from_float = parse_float(2147483647.0) catch:
    then -1
;

min_from_float = parse_float(-2147483648.0) catch:
    then 1
;

[: runtime i32 boundaries: [max_from_string]/[min_from_string]/[max_from_float]/[min_from_float]]

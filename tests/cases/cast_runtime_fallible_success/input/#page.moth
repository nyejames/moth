parse_int |text String| -> Int, Error!:
    return cast! text
;

parse_float |text String| -> Float, Error!:
    return cast! text
;

parse_bool |text String| -> Bool, Error!:
    return cast! text
;

parse_char |text String| -> Char, Error!:
    return cast! text
;

float_to_int |value Float| -> Int, Error!:
    return cast! value
;

int_to_char |value Int| -> Char, Error!:
    return cast! value
;

i Int = parse_int("42") catch:
    then -1
;
f Float = parse_float("3.5") catch:
    then 0.0
;
b Bool = parse_bool("true") catch:
    then false
;
c Char = parse_char("X") catch:
    then ' '
;
fi Int = float_to_int(-7.8) catch:
    then 0
;
ic Char = int_to_char(90) catch:
    then ' '
;

[: runtime fallible casts: [i]|[f]|[b]|[c]|[fi]|[ic]]

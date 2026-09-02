parse_float |text String| -> Float, Error!:
    return cast! text
;

safe |text String| -> String:
    value Float = parse_float(text) catch: then -999.0;
    return cast value
;

[: [safe("1E6")] / [safe("NaN")] / [safe("Infinity")] / [safe("-Infinity")] / [safe("+1.0")] / [safe(" 1.0")] / [safe("1.0 ")] / [safe(".5")] / [safe("1.")] / [safe("1e")] / [safe("1e+")] / [safe("1__0")] / [safe("1e_2")] / [safe("1e10000")]]

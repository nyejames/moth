-- Fallback path inside a match arm body.

get_value |n Int| -> String, Error!:
    if n is 2:
        return! Error("fail")
    ;
    return "ok"
;

selector ~= 2
value ~= ""

if selector is:
    1 =>
        value = get_value(1) catch:
            then "arm1_fallback"
        ;
    2 =>
        value = get_value(2) catch:
            then "arm2_fallback"
        ;
    else =>
        value = get_value(3) catch:
            then "else_fallback"
        ;
;

io.line([: [value]])

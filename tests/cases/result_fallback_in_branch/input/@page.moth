-- Fallback paths inside if-branch and else-branch bodies.

get_value |i Int| -> String, Error!:
    if i is 2:
        return! Error("fail")
    ;
    return "ok"
;

i ~= 0
loop i < 4:
    value ~= ""
    if i is 2:
        value = get_value(i) catch:
            then "branch_fallback"
        ;
    else
        value = get_value(i) catch:
            then "else_fallback"
        ;
    ;
    io.line([: [value]])
    i = i + 1
;

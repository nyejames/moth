-- Loop + match + result fallback combined inside a dispatcher path.

step_value |i Int| -> Int, Error!:
    if i is 2:
        return! Error("fail")
    ;
    return i * 10
;

i ~= 0
sum ~= 0

loop i < 4:
    value = step_value(i) catch:
        then 5
    ;
    if value is:
        0 => sum = sum + 1
        10 => sum = sum + 10
        20 if i > 0 => sum = sum + 20
        30 => sum = sum + 30
        else => sum = sum + 100
    ;

    i = i + 1
;

[:dispatcher_loop_match_result_fallback sum=[sum]]

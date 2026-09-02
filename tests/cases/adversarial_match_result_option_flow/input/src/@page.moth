pair_opt || -> String?, Int?:
    return none, none
;

step_value |idx Int| -> Int, Error!:
    if idx is 2:
        return! Error("boom")
    ;

    return idx + 1
;

i ~= 0
sum ~= 0

loop i < 4:
    name String?, count Int? = pair_opt()

    value = step_value(i) catch:

        then 0

    ;
    if value is:
        0 => sum = sum + 50
        2 if i < 2 => sum = sum + 20
        4 => sum = sum + 4
        else => sum = sum + 1
    ;

    if i is 3:
        i = i + 1
    else
        i = i + 1
    ;
;

[:adversarial_match_result_option_flow sum=[sum]]

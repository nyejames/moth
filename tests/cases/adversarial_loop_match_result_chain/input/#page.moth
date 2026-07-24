read_value |flag Bool| -> Int, Error!:
    if flag:
        return 10
    ;

    return! Error("no value")
;

i ~= 0
sum ~= 0

loop i < 4:
    value = read_value(i is not 2) catch:
        then 1
    ;
    if value is:
        10 => sum = sum + value
        else => sum = sum + 2
    ;

    i = i + 1
;

io.line([: adversarial_loop_match_result_chain final-total=[sum]])

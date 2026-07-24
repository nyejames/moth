-- Dispatcher regression: loop + literal match should use switch-based CFG fallback.

sum ~= 0
counter ~= 0

loop counter < 5:
    if counter is:
        0 => sum = sum + 100
        1 => sum = sum + 10
        else => sum = sum + 1
    ;

    counter = counter + 1
;

io.line([: [sum]])

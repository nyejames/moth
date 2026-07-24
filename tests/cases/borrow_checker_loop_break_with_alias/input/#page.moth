data = "loop_value"
found ~= false
counter ~= 0

loop counter < 5:
    if counter is 2:
        alias = data
        io.line([: borrow_checker_loop_break_with_alias aliased-loop-value=[alias]])
        found = true
        break
    ;
    counter = counter + 1
;

io.line([: borrow_checker_loop_break_with_alias final-found=[found]])

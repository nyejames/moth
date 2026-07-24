data = "shared"
counter ~= 0

loop counter < 3:
    if counter is 1:
        alias = data
        io.line([: adversarial_borrow_nested_loop_aliases alias-path=[alias]])
    else
        copied = copy(data)
        io.line([: adversarial_borrow_nested_loop_aliases copied-path=[copied]])
    ;

    counter = counter + 1
;

final = data
io.line([: adversarial_borrow_nested_loop_aliases final-source=[final]])

outer = {1, 2, 3}
inner = {1, 2, 3}
total ~= 0

loop outer |outer_item, outer_index|:
    loop inner |inner_item, inner_index|:
        if inner_index < 1:
            continue
        ;

        if inner_item > 2:
            break
        ;

        total = total + outer_item + inner_item
    ;

    if outer_index > 1:
        break
    ;
;

[:loop_nested_break_continue total=[total]]

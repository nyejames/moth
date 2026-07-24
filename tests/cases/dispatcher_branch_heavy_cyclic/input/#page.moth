-- Branch-heavy cyclic CFG with multiple independent if branches and a nested match.

i ~= 0
label ~= "start"

loop i < 6:
    if i is 1:
        label = "one"
    else
        if i is 2:
            label = "two"
        else
            label = "other"
        ;
    ;

    if i is 3:
        if label is:
            "two" => label = "matched_two"
            else => label = "matched_other"
        ;
    ;

    if i is 5:
        label = "done"
        break
    ;

    i = i + 1
;

io.line([: [label]])

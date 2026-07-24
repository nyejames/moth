-- Nested loops with cross-level break and continue.

outer ~= {1, 2, 3}
inner ~= {1, 2, 3}
total ~= 0

loop outer |o, oi|:
    loop inner |i, ii|:
        if ii < 1:
            continue
        ;
        if i > 2:
            break
        ;
        total = total + o + i
    ;
    if oi > 1:
        break
    ;
;

[:dispatcher_nested_loops_multi_break total=[total]]

-- Body-local compile-time constant referencing a runtime parameter must fail.

helper |param Int| -> Int:
    value #= param + 1
    return value
;

[:[helper(1)]]

-- Positive: nested map values.

read_a |m {String = {String = Int}}| -> Int:
    a = m.get("a") catch:
        then {}
    ;
    return a.get("x") catch:
        then 0
    ;
;

read_b |m {String = {String = Int}}| -> Int:
    b = m.get("b") catch:
        then {}
    ;
    return b.get("z") catch:
        then 0
    ;
;

outer ~{String = {String = Int}} = {"a" = {"x" = 1, "y" = 2}, "b" = {"z" = 3}}
[:nested_map_values a=[read_a(outer)] b=[read_b(outer)]]

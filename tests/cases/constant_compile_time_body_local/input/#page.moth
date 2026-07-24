-- Body-local compile-time constants with cross-reference.
-- Top-level statements live in the implicit start body, while function
-- declarations prove the same binding mode in explicit function bodies.

make_value || -> Int:
    local_base #= 20
    local_computed #= local_base + 1
    return local_computed
;

base #= 10
computed #= base + 5

[:[computed]-[make_value()]]

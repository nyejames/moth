-- Acyclic match with converging arms that must stay structured.

classify |n Int| -> String:
    label ~= ""
    if n is:
        0 => label = "zero"
        1 => label = "one"
        2 => label = "two"
        else => label = "many"
    ;
    return label
;

result = classify(5)
[:structured_match_converge result=[result]]

capacity #Int = 4
larger_capacity #Int = capacity + 2

render || -> String:
    items ~{larger_capacity String} = {"a", "b", "c", "d", "e", "f"}
    first = items.get(0) catch:
        then ""
    ;
    len = items.length()
    return [:fixed_collection_named_constant_arithmetic_success first=[first] length=[len]]
;

[:[render()]]

render_shorthand || -> String:
    capacity #Int = 4
    names {capacity} = {"a", "b"}
    first = names.get(0) catch:
        then ""
    ;
    len = names.length()
    return [:fixed_collection_shorthand_success first=[first] length=[len]]
;

[:[render_shorthand()]]

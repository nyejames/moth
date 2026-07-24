import @./sizes { capacity }

render_facade || -> String:
    items ~{capacity String} = {}
    ~items.push("w") catch:
    ;
    ~items.push("x") catch:
    ;
    ~items.push("y") catch:
    ;
    ~items.push("z") catch:
    ;
    ~items.push("extra") catch:
    ;
    first = items.get(0) catch:
        then ""
    ;
    len = items.length()
    return [:fixed_collection_facade_capacity_success first=[first] length=[len]]
;

[:[render_facade()]]

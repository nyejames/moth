import @./sizes { capacity }

render_imported || -> String:
    items ~{capacity String} = {}
    ~items.push("a") catch:
    ;
    ~items.push("b") catch:
    ;
    ~items.push("c") catch:
    ;
    ~items.push("d") catch:
    ;
    ~items.push("e") catch:
    ;
    first = items.get(0) catch:
        then ""
    ;
    len = items.length()
    return [:fixed_collection_imported_capacity_success first=[first] length=[len]]
;

[:[render_imported()]]

render_const_capacity || -> String:
    capacity #Int = 4
    items ~{capacity Int} = {}
    ~items.push(9) catch:
    ;
    first = items.get(0) catch:
        then 0
    ;
    len = items.length()
    return [:fixed_collection_const_capacity_success length=[len] value=[first]]
;

[:[render_const_capacity()]]

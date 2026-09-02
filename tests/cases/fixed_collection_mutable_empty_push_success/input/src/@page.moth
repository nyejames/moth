render_mutable_push || -> String:
    items ~{2 Int} = {}
    ~items.push(1) catch:
    ;
    ~items.push(2) catch:
    ;
    first = items.get(0) catch:
        then 0
    ;
    second = items.get(1) catch:
        then 0
    ;
    len = items.length()
    return [:fixed_collection_mutable_empty_push_success length=[len] first=[first] second=[second]]
;

[:[render_mutable_push()]]

render_remove || -> String:
    items ~{2 Int} = {1, 2}
    removed = ~items.remove(0) catch:
        then 0
    ;
    ~items.push(3) catch:
    ;
    first = items.get(0) catch:
        then 0
    ;
    second = items.get(1) catch:
        then 0
    ;
    len = items.length()
    return [:fixed_collection_remove_preserves_capacity removed=[removed] first=[first] second=[second] length=[len]]
;

[:[render_remove()]]

push_overflow_message |items ~{1 Int}| -> String:
    ~items.push(2) catch |err|:
        return err.message
    ;

    return "not caught"
;

render_overflow || -> String:
    items ~{1 Int} = {}
    ~items.push(1) catch:
    ;
    message = push_overflow_message(~items)
    len = items.length()
    first = items.get(0) catch:
        then 0
    ;
    return [:fixed_collection_push_overflow_catch message=[message] length=[len] first=[first]]
;

[:[render_overflow()]]

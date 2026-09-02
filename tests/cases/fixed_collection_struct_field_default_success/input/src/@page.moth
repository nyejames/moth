Buffer = |
    items {4 Int} = {},
|

render_struct_field || -> String:
    buffer ~= Buffer()
    ~buffer.items.push(1) catch:
    ;
    first = buffer.items.get(0) catch:
        then 0
    ;
    len = buffer.items.length()
    return [:fixed_collection_struct_field_default_success length=[len] value=[first]]
;

[:[render_struct_field()]]

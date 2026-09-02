render_nested || -> String:
    rows #Int = 2
    cols #Int = 3
    grid {rows {cols Int}} = {{1}, {2}}
    second = grid.get(1) catch:
        then {0}
    ;
    value = second.get(0) catch:
        then 0
    ;
    grid_len = grid.length()
    return [:nested_fixed_collection_success value=[value] rows=[grid_len]]
;

[:[render_nested()]]

capacity #Int = 4
Names as {capacity String}

render_alias || -> String:
    names Names = {"a"}
    first = names.get(0) catch:
        then ""
    ;
    len = names.length()
    return [:fixed_collection_alias_success first=[first] length=[len]]
;

[:[render_alias()]]

render_alias_observes || -> String:
    values ~= {1, 2}
    ~values.push(3) catch:
    ;
    alias = values
    len = alias.length()
    return [:collection_alias_observes_mutation len=[len]]
;

[:[render_alias_observes()]]

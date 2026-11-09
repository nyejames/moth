render_branch_write || -> String:
    values ~= {10, 20, 30}
    if true:
        ~values.set(1, 99) catch:
        ;
    ;
    second = values.get(1) catch:
        then 0
    ;
    return [:collection_set_readback_branch second=[second]]
;

[:[render_branch_write()]]

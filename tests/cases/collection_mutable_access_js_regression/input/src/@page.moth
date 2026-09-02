render_js_regression || -> String:
    values ~= {1}
    ~values.push(2)
    ~values.set(0, 9) catch:
    ;
    ~values.set(1, 99) catch:
    ;
    removed = ~values.remove(0) catch:
        then 0
    ;
    first = values.get(0) catch:
        then 0
    ;
    len = values.length()
    return [:collection_mutable_access_js_regression first=[first] removed=[removed] len=[len]]
;

[:[render_js_regression()]]

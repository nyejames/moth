-- Success-path coverage for all collection helpers with strict runtime contract.

render_strict_contract || -> String:
    values ~= {10, 20, 30}
    ~values.push(40) catch:
    ;
    ~values.set(0, 99) catch:
    ;
    removed = ~values.remove(1) catch:
        then 0
    ;
    first = values.get(0) catch:
        then 0
    ;
    len = values.length()
    return [:strict_contract first=[first] removed=[removed] len=[len]]
;

[:[render_strict_contract()]]

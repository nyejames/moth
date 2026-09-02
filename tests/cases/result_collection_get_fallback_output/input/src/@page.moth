-- Prove that collection.get result handling works through final output.
-- Tests both success branch (in-bounds) and fallback (out-of-bounds).

get_with_fallback || -> String:
    items ~= {10, 20, 30}
    in_bounds = items.get(1) catch:
        then 0
    ;
    out_of_bounds = items.get(99) catch:
        then 42
    ;
    return [:collection_get_fallback in_bounds=[in_bounds] out_of_bounds=[out_of_bounds]]
;

[:[get_with_fallback()]]

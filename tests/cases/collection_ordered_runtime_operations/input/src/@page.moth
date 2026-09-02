-- Ordered growable-collection runtime operations. One primary owner covers
-- `language.collections.ordered_runtime_operations`: non-empty collection
-- literal construction plus ordered push, set, and remove runtime behavior,
-- with every remaining element read back and the removed value and final
-- length/order reported. Each dimension is named and independently observable
-- through one top-level runtime output event.

render_ordered_runtime_operations || -> String:
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
    second = values.get(1) catch:
        then 0
    ;
    third = values.get(2) catch:
        then 0
    ;
    len = values.length()
    return [:collection_ordered_runtime_operations removed=[removed] first=[first] second=[second] third=[third] len=[len]]
;

[:[render_ordered_runtime_operations()]]

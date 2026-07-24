-- Backend-shape contract case: assert helper/lowering shape, not collection semantics.

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

[:collection_methods_backend_contract first=[first] removed=[removed] len=[len]]

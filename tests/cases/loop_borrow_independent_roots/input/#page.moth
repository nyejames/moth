items ~{Int} = {1, 2, 3}
other ~{Int} = {4, 5}
copied ~= copy items

loop items |item|:
    ~other.push(item) catch:
    ;
    ~copied.push(item) catch:
    ;
;

[:loop_borrow_independent_roots original=[items.length()] other=[other.length()] copied=[copied.length()]]

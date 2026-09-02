-- Borrow interaction: mutating the source collection while iterating it.
items ~= {1, 2, 3}

loop items |iter_value|:
    ~items.push(4) catch:
    ;
;

io.line([: [items.length()]])

-- AST rejects multiple-return value passed to io.line([: []])

multi || -> String, Int:
    return "hello", 42
;

io.line(multi())

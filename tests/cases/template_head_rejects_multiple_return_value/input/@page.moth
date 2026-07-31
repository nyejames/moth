-- AST rejects multiple-return value in template head expression

multi || -> String, Int:
    return "hello", 42
;

[: [multi()]]

import @helper { Counter }

run || -> String:
    c = Counter(0)
    result = c.tick()
    return "source_visibility_ok"
;

io.line([: [run()]])

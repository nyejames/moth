import @helper { Counter }

run || -> String:
    c = Counter(41)
    result = c.tick()
    return "duplicate_receiver_alias_ok"
;

io.line([: [run()]])

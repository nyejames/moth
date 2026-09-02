-- Multiple break and continue targets inside a single loop.

count ~= 0
i ~= 0

loop i < 10:
    if i is 1:
        i = i + 1
        continue
    ;

    if i is 5:
        i = i + 1
        break
    ;

    if i is 3:
        i = i + 1
        continue
    ;

    count = count + 1
    i = i + 1
;

io.line([: [count]])

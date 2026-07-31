import @web/canvas

run |id String| -> String, Error!:
    canvas_ref = canvas.get_canvas(id)!
    ctx = canvas.context_2d(canvas_ref)!
    return "ok"
;

result = run("game") catch:
    then "error"
;
io.line([: [result]])

import @web/canvas { Canvas2d, get_canvas, context_2d, fill_rect }

run |id String| -> String, Error!:
    canvas_ref = get_canvas(id)!
    ctx ~Canvas2d = context_2d(canvas_ref)!
    fill_rect(~ctx, 0.0, 0.0, 24.0, 24.0)
    return "with_type_ok"
;

result = run("game") catch:
    then "error"
;
io.line([: [result]])

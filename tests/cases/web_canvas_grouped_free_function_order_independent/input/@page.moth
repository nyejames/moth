import @web/canvas { fill_rect, context_2d, get_canvas, Canvas2d }

run |id String| -> String, Error!:
    canvas_ref = get_canvas(id)!
    ctx ~Canvas2d = context_2d(canvas_ref)!
    fill_rect(~ctx, 0.0, 0.0, 18.0, 18.0)
    return "order_independent_ok"
;

result = run("game") catch:
    then "error"
;
io.line([: [result]])

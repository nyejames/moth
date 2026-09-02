Status ::
    Ready,
    Waiting,
;

score_for |status Status| -> Int:
    score = if status is Ready then 1 else 0
    return score
;

qualified_score_for |status Status| -> Int:
    score = if status is Status::Ready then 1 else 0
    return score
;

io.line("value_if_inline_choice_predicate")
bare_ready = score_for(Status::Ready)
qualified_waiting = qualified_score_for(Status::Waiting)
io.line([:bare ready [bare_ready]])
io.line([:qualified waiting [qualified_waiting]])

import @core/time {
    TimeMark,
    Duration,
    mark_now,
    elapsed_since,
    duration_between,
    as_milliseconds,
}

-- Reduce live time to stable ok/bad output
mark1 = mark_now()
mark2 = mark_now()

delta = duration_between(mark1, mark2)
elapsed = elapsed_since(mark1)

-- Both deltas should be non-negative
delta_ok = as_milliseconds(delta) >= 0.0
elapsed_ok = as_milliseconds(elapsed) >= 0.0

io.line([:mark_delta delta_ok=[delta_ok] elapsed_ok=[elapsed_ok]])

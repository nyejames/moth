import @core/time {
    Duration,
    duration_from_seconds as seconds,
    duration_from_milliseconds as millis,
    as_seconds as seconds_value,
    as_milliseconds as millis_value,
}

-- Aliased free functions
d1 = seconds(1.5)
d2 = millis(500.0)

sec = seconds_value(d1)
m = millis_value(d2)

io.line([:alias sec=[sec] millis=[m]])

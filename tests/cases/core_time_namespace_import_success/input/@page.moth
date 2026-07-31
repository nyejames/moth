import @core/time

-- Namespace value access for free functions
d = time.duration_from_seconds(1.0)
sec = time.as_seconds(d)

-- Namespace type-position access if supported
value time.Duration = time.duration_from_seconds(2.0)
millis = time.as_milliseconds(value)

io.line([:namespace sec=[sec] millis=[millis]])

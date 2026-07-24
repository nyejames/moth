import @core/time {Duration, Timestamp, duration_from_seconds, timestamp_now}

d = duration_from_seconds(1.0)
-- Attempt to access a field on an external opaque type
bad1 = d.millis

ts = timestamp_now()
bad2 = ts.seconds

io.line([: [bad1]])
io.line([: [bad2]])

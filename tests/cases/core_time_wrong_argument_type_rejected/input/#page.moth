import @core/time {Duration, TimeMark, Timestamp, duration_from_seconds, elapsed_since, timestamp_from_unix_seconds}

-- Pass String where Float is required for duration construction
bad1 = duration_from_seconds("one")

-- Pass String where TimeMark is required
bad2 = elapsed_since("mark")

-- Pass String where the timestamp constructor requires Float input
bad3 = timestamp_from_unix_seconds("epoch")

io.line([: [bad1]])

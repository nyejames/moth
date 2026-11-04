AlertLevel ::
    Ok,
    Watch,
    Warning,
    Critical,
;

format_level |level AlertLevel| -> String:
    if level is:
        Ok => return "ok"
        Watch => return "watch"
        Warning => io.line("warning")
        Critical => return "critical"
    ;
;

message = format_level(AlertLevel::Warning)
io.line([: [message]])

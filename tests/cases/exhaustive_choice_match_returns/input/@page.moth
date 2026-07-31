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
        Warning => return "warning"
        Critical => return "critical"
    ;
;

message = format_level(AlertLevel::Warning)
[:exhaustive_choice_match_returns message=[message]]

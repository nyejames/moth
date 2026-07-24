import @types/enums {Priority}
import @utils/formatting {format_label}

render_badge |level Priority| -> String:
    if level is Priority::Critical:
        return [: <span class="badge critical">[format_label("critical")]</span>]
    ;
    if level is Priority::High:
        return [: <span class="badge high">[format_label("high")]</span>]
    ;
    return [: <span class="badge normal">[format_label("normal")]</span>]
;

import @types/models {User, UserProfile}
import @types/enums {Status, Priority}
import @core/constants {max_items_per_page}
import @utils/formatting {format_label}
import @widgets/card {Card}
import @widgets/badge {render_badge}

render_home |user User| -> String:
    if user.status is Status::Active:
        return [: Active user [user.username] with [max_items_per_page] items]
    ;
    return [: Inactive user [user.username]]
;

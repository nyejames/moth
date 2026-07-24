import @types/models {User}
import @core/constants {app_name, app_version}
import @utils/formatting {format_label}

render_contact |user User| -> String:
    return [: Contact [user.username] at [user.email] via [format_label(app_name)] v[app_version]]
;

import @types/models {UserProfile}
import @types/enums {Priority}
import @core/constants {app_version, build_timestamp}
import @utils/formatting {format_timestamp}
import @widgets/badge {render_badge}

render_about |profile UserProfile| -> String:
    return [:
        <div class="about">
            <h2>[profile.display_name]</h2>
            <p>Role: [profile.role]</p>
            <p>Version: [app_version]</p>
            <p>Built: [format_timestamp(build_timestamp)]</p>
            [render_badge(profile.priority)]
        </div>
    ]
;

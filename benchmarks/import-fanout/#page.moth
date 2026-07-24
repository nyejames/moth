import @core/constants {app_name, app_version}
import @core/constants {build_timestamp as built_at}
import @types/models {User, UserProfile}
import @types/enums {Status, Priority}
import @utils/formatting {format_label, format_timestamp}
import @utils/math {clamp}
import @widgets/header {render_header}
import @widgets/footer {render_footer}
import @widgets/card {render_card, Card}
import @pages/home {render_home}
import @pages/about {render_about}

page_title #= [: [app_name] v[app_version]]

user ~= User("admin", "admin@example.com", Status::Active)
profile ~= UserProfile("Admin User", "Administrator", Priority::High)

card_a = Card(format_label("Home"), render_home(user))
card_b = Card(format_label("About"), render_about(profile))

[:
    [render_header(page_title)]
    <main>
        [render_card(card_a)]
        [render_card(card_b)]
    </main>
    [render_footer(built_at)]
]

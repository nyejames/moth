import @core/constants {app_name, version}
import @widgets/card {Card, render_card}
import @widgets/badge {render_badge}
import @util/formatting {format_label}
import @pages/home {render_home}
import @pages/about {render_about}

page_title #= "Module Root Stress"

card_a = Card(format_label("Home"), render_home())
card_b = Card(format_label("About"), render_about())

[:
    <main>
        <h1>[page_title]</h1>
        [render_card(card_a)]
        [render_card(card_b)]
        [render_badge("stress")]
    </main>
]

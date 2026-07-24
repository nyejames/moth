import @components/card {Card, render_card}
import @components/listing {render_listing}
import @data/constants {app_title}

render_page |card Card| -> String:
    rendered_card = render_card(card)
    cards = {card, Card("Nested", "Grouped import path")}
    return [:
        <main>
            <h1>[app_title]</h1>
            [rendered_card]
            [render_listing(cards)]
        </main>
    ]
;

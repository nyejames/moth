import @data/theme {default_theme, accent}
import @util/formatting {render_badge as badge}

Card = |
    title String,
    body String,
|

render_card |card Card| -> String:
    return [:
        <article style="border-color: [accent]; padding: [default_theme.spacing]px">
            <h2>[card.title]</h2>
            [badge("module")]
            <p>[card.body]</p>
        </article>
    ]
;


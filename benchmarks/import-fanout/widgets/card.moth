import @utils/formatting {format_label}

Card = |
    title String,
    body String,
|

render_card |card Card| -> String:
    return [:
        <article class="card">
            <h2>[format_label(card.title)]</h2>
            <p>[card.body]</p>
        </article>
    ]
;

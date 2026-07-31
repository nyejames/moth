import @html {img}

card_image_css #= "object-fit: cover; border-radius: 999px; max-width: 64px;"

[$html:
    <section class="runtime-slot-test">
        [img, @assets/test-card.webp:
            [$insert("style"):[card_image_css]]
            Runtime wrapper slot child survived
        ]

        <p>Runtime parent template survived</p>
    </section>
]

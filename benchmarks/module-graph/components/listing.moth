import @components/card {Card, render_card}
import @util/mathish {scale_spacing}

render_listing |cards {Card}| -> String:
    output ~= [: <section data-gap="[scale_spacing(2)]">]
    loop cards |card|:
        output = [: [output][render_card(card)]]
    ;
    return [: [output]</section>]
;


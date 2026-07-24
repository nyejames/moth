import @./theme { accent }

[: imported-root-runtime-must-not-execute ]

private_prefix #= "facade"

PrivateLabel = |
    text String,
|

make_private_label |text String| -> PrivateLabel:
    return PrivateLabel([: [private_prefix]-[text]-[accent]])
;

export:
    PublicCard = |
        title String,
    |

    PublicState ::
        Ready,
    ;

    PublicText as String

    public_title #= "card"

    make_state || -> PublicState:
        return PublicState::Ready
    ;

    render_card |card PublicCard| -> PublicText:
        label = make_private_label(card.title)
        return label.text
    ;
;

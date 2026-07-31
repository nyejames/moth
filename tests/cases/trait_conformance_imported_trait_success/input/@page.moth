import @./traits { RENDERABLE }

Card = |
    title String,
|

render |this Card| -> String:
    return this.title
;

Card must RENDERABLE

card = Card("imported")
[:trait_conformance_imported_trait_success [card.render()]]

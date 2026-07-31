-- Structural choice equality truth table. One primary owner covers the whole
-- truth-table contract for a single choice type under
-- `language.choices.structural_equality_truth_table`: unit equality, same and
-- different unit variants, same and different scalar payloads, unit-versus-
-- payload inequality, distinct payload-variant inequality, and nested choice
-- equality and inequality. Each dimension is named and independently observable
-- through one top-level runtime output event.

Inner ::
    A | value Int |,
    B,
;

Outer ::
    First,
    Second,
    Word | text String |,
    Count | number Int |,
    Wrapped | inner Inner |,
;

unit_same = Outer::First is Outer::First
unit_different = Outer::First is Outer::Second
word_same_payload = Outer::Word("hi") is Outer::Word("hi")
word_different_payload = Outer::Word("hi") is Outer::Word("bye")
unit_vs_payload = Outer::First is Outer::Word("hi")
different_payload_variants = Outer::Word("hi") is Outer::Count(7)
nested_same = Outer::Wrapped(Inner::A(1)) is Outer::Wrapped(Inner::A(1))
nested_different = Outer::Wrapped(Inner::A(1)) is Outer::Wrapped(Inner::A(2))

[:choice_equality_truth_table unit_same=[unit_same] unit_different=[unit_different] word_same_payload=[word_same_payload] word_different_payload=[word_different_payload] unit_vs_payload=[unit_vs_payload] different_payload_variants=[different_payload_variants] nested_same=[nested_same] nested_different=[nested_different]]

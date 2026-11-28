; Initial single-text-chunk experiments, not production range grouping.
; Do not set injection.combined: unrelated template bodies must stay separate.
((construction
  (construction_head (formatter_directive "$html"))
  (template_body (template_text) @injection.content))
 (#set! injection.language "html"))

((construction
  (construction_head (formatter_directive "$css"))
  (template_body (template_text) @injection.content))
 (#set! injection.language "css"))

((construction
  (code_directive (string) @_language)
  (listing_body) @injection.content)
 (#eq? @_language "\"moth\"")
 (#set! injection.language "moth")
 (#set! injection.include-children))

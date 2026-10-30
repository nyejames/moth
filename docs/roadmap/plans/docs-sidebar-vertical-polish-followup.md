# Documentation sidebar vertical layout polish

Status: queued follow-up on `web-site-and-page-v1`
Prerequisite: complete and validate the HTML JavaScript glue-sharing and web package hardening work through final audit and handoff.
Scope: documentation-site sidebar UI only. Do not change the public `@web/page` or `@web/site` APIs, HTML builder JavaScript deduplication, general DOM bindings or mobile navigation semantics.

## Goal

Improve the wide-screen documentation sidebar in three ways:

1. Make the entire collapsed vertical sidebar the expand control, with the hamburger centred horizontally and vertically.
2. Allow the sidebar to rise into the space vacated by the scrolling primary navbar, expanding its usable height while keeping its bottom spacing fixed.
3. On initial navigation and on expansion, scroll the sidebar to the category heading containing the current page, clamping naturally at the end of the navigation list.

All three behaviours apply only to the desktop vertical sidebar at widths greater than 960px. The existing horizontal mobile sidebar stays unchanged.

## Current implementation

The shared component lives in `docs/src/styles/sidebar.moth`. It uses a visually hidden checkbox (`.docs-sidebar__toggle`), a linked label (`.docs-sidebar__button`) and an aside (`.docs-sidebar`). The desktop aside currently uses `position: fixed`, `top: 4.5rem`, `bottom: 1rem` and `overflow-y: auto`. The collapsed state reduces its width to `3.75rem` and hides the breadcrumb and navigation. Its label remains a `2.25rem` square at the top.

The primary navbar is in `docs/src/styles/navbar.moth`, with the existing `nav[aria-label="Primary"]` markup. The sidebar is rendered after the navbar on docs pages.

## Phase 1 - desktop collapsed hit area

- Preserve the native checkbox and label interaction. Do not introduce an overlay button or duplicate toggle state.
- In the desktop collapsed state only, make the existing `.docs-sidebar__button` label cover the complete visible aside, including the top, bottom and side padding. Anchor it to the aside with absolute positioning. Centre its icon in both dimensions.
- Give the whole hit area a pointer cursor and a subtle light/dark hover treatment. Keep the visible icon and the collapsed width unchanged unless measured layout inspection reveals a real need.
- Ensure the collapsed label respects the aside's rounded shape. Avoid scrollbars and clipped or invisible focus indicators in the collapsed state.
- Preserve checkbox keyboard interaction and a clear `:focus-visible` indicator. In the expanded state, the button returns to its existing header position and size.
- Keep every new selector inside the `min-width: 961px` media query. Mobile expanded/collapsed behaviour and print layout must remain exactly as before.

A CSS-only solution should be sufficient. Prefer existing selectors, style constants and checkbox state rather than new nodes or JavaScript.

## Phase 2 - responsive top clearance on scroll

The intended geometry on desktop is:

```text
sidebar_top = max(1rem, navbar_viewport_bottom + 1rem)
sidebar_bottom = 1rem
```

Here `navbar_viewport_bottom` is the navbar's current bottom edge relative to the viewport, including its movement out of view as the document scrolls. At scroll position zero, the sidebar remains below the navbar. As the navbar disappears, the sidebar continuously rises, stopping at `1rem` from the viewport top. Its fixed bottom edge makes the available sidebar height grow by exactly the freed space.

- Keep `.docs-sidebar` fixed positioned, with its current left placement, widths, bottom offset and independent vertical scrolling.
- Replace the hard-coded fixed top with a desktop CSS custom-property calculation. Preserve `top: 4.5rem` as an acceptable no-JavaScript fallback.
- Supply the live navbar bottom edge through one small docs-only browser script, unless a simpler solution proves correct across the supported browsers. Use `getBoundingClientRect().bottom` and update the custom property, throttled to one `requestAnimationFrame` per scroll/resize frame. Make initial layout correct on page load and after browser scroll restoration.
- The script must use the existing navbar and sidebar elements and be loaded once per page. Prefer one source-controlled, linked docs asset over repeated inline scripts. Verify it passes the HTML builder's normal resource and output policies.
- Do not add a public `@web/page` method, a generic event system, a new JavaScript emitter, an interval/polling loop or a dependency.
- Do not add a CSS `transition` to the scrolling `top` or resulting height. These changes should track the document scroll directly without lag. Existing hover transitions may remain. Respect reduced-motion preferences for any new decorative animation.
- Keep the existing `max-width: 960px` layout independent of desktop top-clearance styles. The script should have no visible effect there.

The already accepted docs navigation initializer (`init_docs_sidebar()`) continues to own active-link and breadcrumb synchronisation. The new scroll behaviour must not modify `@web/page`'s `sync_navigation` or its existing no-listener contract.

## Phase 3 - align navigation to the current category

On desktop, position the sidebar's independent scroll so that the preceding category heading for the active page is at the top of the scrollport. For example, Loops and Wiring should begin at `Learn Moth`; Core packages should begin at `Projects and content`. The browser should clamp naturally if there is not enough material after the heading to reach that position. Do not attempt additional automatic link-reveal scrolling or split categories programmatically.

- Reuse the existing `aria-current="page"` link selected by `page.sync_navigation("docs-sidebar")`. Do not independently resolve routes, add page-specific group metadata or duplicate the matching logic in JavaScript. The current nav's `.docs-sidebar__group` headings and `.docs-sidebar__link` siblings provide the group relationship. Walk backwards from the marked link to its nearest preceding heading; if no marked link or preceding heading exists, leave scroll position alone.
- Calculate the heading's scroll target *relative to `.docs-sidebar`*, which is the desktop scroll container, then set its `scrollTop`. `getBoundingClientRect()` differences plus the current `scrollTop` are robust to offset-parent differences; account for any intended top padding. Native scroll bounds handle the maximum possible offset. Avoid `scrollIntoView()`, which can also scroll the document itself.
- On initial page navigation, position after the existing sidebar navigation initializer marks the link. Establish a reliable ordering using the page's established startup lifecycle, e.g. a `pageshow`/load callback after the deferred module initializer where verified. Avoid a timer-based race, DOM polling, a `MutationObserver` or changes to the public `@web/page` API. The docs root/homepage may have no matching active link, which must be a normal no-op.
- On desktop checkbox expansion (checked to unchecked), wait until the collapsed navigation is rendered again, using one animation frame as needed, then recompute the active heading and scroll target. Do not measure a `display: none` nav. An ordinary collapse doesn't need a scroll update.
- Keep this logic in the same small docs-only browser asset as the navbar-clearance behaviour from Phase 2. Reuse any existing listeners/setup rather than adding duplicate initialisation. No state persistence, animation framework, navigation rewriter or new compiler/runtime binding.
- Apply the alignment immediately (`scrollTop` or an equivalent non-animated scroll). A smooth animation on every navigation or expansion would add delay and cause a distracting sweep through the entire sidebar. The changing sidebar position from Phase 2 should still track page scrolling continuously.
- Limit automatic positioning to `min-width: 961px`. Preserve the mobile horizontal dropdown's current scroll and expand semantics, as well as ordinary manual scrolling while the desktop sidebar remains open. In particular, do not rerun this alignment on every document scroll/resize, as that would fight the user.

## Validation

1. At viewport widths 961px and above, verify the collapsed bar expands when clicked near its top, centre, bottom and either edge. The icon stays centred throughout scrolling and resizing.
2. Verify Tab and Space keyboard operation and visible focus indication when the bar is collapsed. Reopen and close without losing checkbox state or changing the mobile button.
3. At the document top, confirm clearance beneath the real navbar. Scroll slowly through the navbar's disappearance and back up. The sidebar must track the scroll continuously, stop with `1rem` top and bottom clearance and not jump or overlap the navbar.
4. In the expanded state, verify that the increased available height actually reveals more navigation and preserves its independent scrolling. Test short and long documents, tall and short viewports and browser zoom.
5. At 960px and below, confirm the horizontal sidebar, toggle, spacing and expanded navigation remain unchanged. Check the 960px/961px breakpoint and print layout.
6. Check page load when already scrolled, viewport resize, refresh and browser back/forward restoration. Confirm no console errors or stale top offset.
7. Verify automatic category positioning on initial navigation and expansion for Loops/Wiring (Learn Moth), Core packages (Projects and content), a Reference page and a Developer docs page. Check a short viewport where the category cannot reach the top, an unlisted page with no active link and cases where the navigation starts collapsed. The browser document scroll must not change. Manual scrolling of the open sidebar must not be reset by ordinary page scrolling or resizing.
8. Run the repository's focused docs tests, `moth check docs --terse`, `moth build docs --release` and applicable validation gates. Inspect generated pages at the site root, documentation root, nested docs pages and developer-docs routes. Verify a shared JavaScript asset, if added, is linked correctly at different route depths and not emitted redundantly.
9. Review the final diff for duplicate listeners, extra abstractions, unnecessary markup and stale style rules. Keep this follow-up small and isolated.

## Acceptance

- The whole collapsed desktop bar is an accessible expand target, with a centred icon.
- The desktop sidebar smoothly occupies the navbar's vacated vertical space and keeps consistent top and bottom viewport clearance.
- Expanded content scrolls independently and aligns the active page's category heading at the top when possible, with natural clamping otherwise. It does not fight subsequent manual scrolling. Mobile, print, breadcrumb and existing checkbox interactions retain their behaviour.
- Changes are confined to docs-site source/assets and associated focused tests. Public web package APIs, compiler-generated glue and builder output semantics are unchanged.
- All applicable checks pass. Record any browser-specific limitation rather than introducing a broad compatibility subsystem.

import {
    mothOk,
    mothErr,
    MOTH_ERROR_HOST_INVALID_ARGUMENT,
    MOTH_ERROR_HOST_RESOURCE_NOT_FOUND,
} from "@moth/runtime";

// This file is the Moth-facing `@web/page` facade over live browser state.
// Every getter reads the browser on each call. Nothing caches import-time values or subscribes
// to changes. The external binding glue checks I32 results and the HIR float boundary checks F64
// finiteness, so getters return the browser values unchanged. `sync_navigation` is an opt-in
// helper for one static navigation tree.

/**
 * @moth.sig url || -> String
 */
export function url() {
    return window.location.href;
}

/**
 * @moth.sig path || -> String
 */
export function path() {
    return window.location.pathname;
}

/**
 * @moth.sig origin || -> String
 */
export function origin() {
    return window.location.origin;
}

/**
 * @moth.sig query || -> String
 */
export function query() {
    return window.location.search;
}

/**
 * @moth.sig fragment || -> String
 */
export function fragment() {
    return window.location.hash;
}

/**
 * @moth.sig title || -> String
 */
export function title() {
    return document.title;
}

/**
 * @moth.sig viewport_width || -> I32
 */
export function viewportWidth() {
    return window.innerWidth;
}

/**
 * @moth.sig viewport_height || -> I32
 */
export function viewportHeight() {
    return window.innerHeight;
}

/**
 * @moth.sig pixel_ratio || -> F64
 */
export function pixelRatio() {
    return window.devicePixelRatio;
}

/**
 * @moth.sig scroll_x || -> F64
 */
export function scrollX() {
    return window.scrollX;
}

/**
 * @moth.sig scroll_y || -> F64
 */
export function scrollY() {
    return window.scrollY;
}

/**
 * @moth.sig is_visible || -> Bool
 */
export function isVisible() {
    return document.visibilityState === "visible";
}

/**
 * @moth.sig has_focus || -> Bool
 */
export function hasFocus() {
    return document.hasFocus();
}

/**
 * @moth.sig prefers_dark || -> Bool
 */
export function prefersDark() {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/**
 * @moth.sig prefers_reduced_motion || -> Bool
 */
export function prefersReducedMotion() {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

// Route keys compare only path identity. Directory aliases share one key: `/a/index.html`,
// `/a/` and `/a` all become `/a`, and the site root becomes "". Percent escapes and case are
// kept, so `%2F` never becomes a separator.
function routeKey(pathname) {
    return pathname.replace(/\/index\.html$/, "/").replace(/\/$/, "");
}

// Returns null for anchors that can't take part in matching: no href, an unparsable href or
// another origin.
function anchorRouteKey(anchor) {
    const href = anchor.getAttribute("href");
    if (href === null) {
        return null;
    }

    try {
        const resolved = new URL(href, document.baseURI);
        return resolved.origin === window.location.origin ? routeKey(resolved.pathname) : null;
    } catch {
        return null;
    }
}

// Ancestors are slash-bounded, so `/docs/pack` never contains `/docs/packages`.
function isAncestorKey(candidate, key) {
    return key.startsWith(candidate + "/");
}

function findNavigationHook(root, hook) {
    return root.querySelector(`[${hook}]`);
}

/**
 * @moth.sig sync_navigation |root_id String| -> Bool, Error!
 */
export function syncNavigation(rootId) {
    if (rootId === "") {
        return mothErr(MOTH_ERROR_HOST_INVALID_ARGUMENT, "Navigation root ID is empty");
    }

    const root = document.getElementById(rootId);
    if (!root) {
        return mothErr(MOTH_ERROR_HOST_RESOURCE_NOT_FOUND, "Navigation root not found: " + rootId);
    }

    const home = findNavigationHook(root, "data-page-home");
    const links = findNavigationHook(root, "data-page-links");
    const current = findNavigationHook(root, "data-page-current");
    for (const [hook, element] of [
        ["data-page-home", home],
        ["data-page-links", links],
        ["data-page-current", current],
    ]) {
        if (!element) {
            return mothErr(MOTH_ERROR_HOST_RESOURCE_NOT_FOUND, "Navigation hook not found: " + hook);
        }
    }

    // Each call rebuilds every managed marker, so a repeated call after a History API change
    // leaves no stale state behind.
    const currentKey = routeKey(window.location.pathname);
    const homeKey = anchorRouteKey(home);
    const candidates = links.querySelectorAll("a[href]");
    home.removeAttribute("aria-current");
    for (const link of candidates) {
        link.removeAttribute("aria-current");
    }

    if (homeKey !== null && currentKey === homeKey) {
        home.setAttribute("aria-current", "page");
        current.replaceChildren();
        current.hidden = true;
        return mothOk(true);
    }

    let match = null;
    const ancestors = new Map();
    for (const link of candidates) {
        const key = anchorRouteKey(link);
        if (key === null) {
            continue;
        }

        if (key === currentKey && match === null) {
            match = link;
        }

        const isNewAncestor = key !== homeKey && isAncestorKey(key, currentKey) && !ancestors.has(key);
        if (isNewAncestor) {
            ancestors.set(key, link);
        }
    }

    if (match) {
        match.setAttribute("aria-current", "page");
    }

    // An unlisted page prefers its authored heading because builder title prefixes and
    // postfixes may decorate `document.title`.
    const heading = match ? null : document.querySelector("h1");
    const label = match ? match.textContent.trim() : heading?.textContent.trim() || document.title;

    // The home anchor is the visible first crumb, so the container holds only the rest. An
    // ancestor repeating the home or final label would read as a duplicate crumb.
    const crumbs = [];
    if (label) {
        const homeLabel = home.textContent.trim();
        // Every ancestor key prefixes the current key, so shorter keys are shallower.
        const orderedAncestors = [...ancestors].sort(([left], [right]) => left.length - right.length);
        for (const [, link] of orderedAncestors) {
            const ancestorLabel = link.textContent.trim();
            if (ancestorLabel === homeLabel || ancestorLabel === label) {
                continue;
            }

            const crumb = document.createElement("a");
            crumb.setAttribute("href", link.getAttribute("href"));
            crumb.textContent = ancestorLabel;
            crumbs.push(document.createTextNode(" / "), crumb);
        }

        const currentCrumb = document.createElement("span");
        currentCrumb.setAttribute("aria-current", "page");
        currentCrumb.textContent = label;
        crumbs.push(document.createTextNode(" / "), currentCrumb);
    }

    current.replaceChildren(...crumbs);
    current.hidden = !label;
    return mothOk(match !== null);
}

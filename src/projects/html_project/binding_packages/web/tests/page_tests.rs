//! Node execution checks for the browser-facing page package.

use super::node_asset::run_asset_in_node;

const PAGE_SOURCE: &str = include_str!("../page/page.js");

#[test]
fn getters_read_live_browser_values() {
    run_asset_in_node(
        "page",
        PAGE_SOURCE,
        r#"
let focused = true;
const queries = [];
let dark = true, motion = false;
globalThis.window = {
    location: new URL('https://example.test/a?q=1#part'), innerWidth: 800, innerHeight: 600,
    devicePixelRatio: 2, scrollX: 1.5, scrollY: -3,
    matchMedia(query) { queries.push(query); return { matches: query === '(prefers-color-scheme: dark)' ? dark : motion }; },
};
globalThis.document = { title: 'First', visibilityState: 'visible', hasFocus: () => focused };
function check(expected) {
    assert.deepEqual([page.url(), page.path(), page.origin(), page.query(), page.fragment(), page.title(),
        page.viewportWidth(), page.viewportHeight(), page.pixelRatio(), page.scrollX(), page.scrollY(),
        page.isVisible(), page.hasFocus(), page.prefersDark(), page.prefersReducedMotion()], expected);
}
check(['https://example.test/a?q=1#part', '/a', 'https://example.test', '?q=1', '#part', 'First', 800, 600, 2, 1.5, -3, true, true, true, false]);
window.location = new URL('https://other.test/b');
Object.assign(window, { innerWidth: 1.5, innerHeight: 42, devicePixelRatio: Infinity, scrollX: -2, scrollY: 8 });
Object.assign(document, { title: 'Second', visibilityState: 'hidden' });
focused = false;
dark = false;
motion = true;
check(['https://other.test/b', '/b', 'https://other.test', '', '', 'Second', 1.5, 42, Infinity, -2, 8, false, false, false, true]);
assert.deepEqual(queries, ['(prefers-color-scheme: dark)', '(prefers-reduced-motion: reduce)', '(prefers-color-scheme: dark)', '(prefers-reduced-motion: reduce)']);
"#,
    );
}

const DOM: &str = r#"
class Element {
    constructor(tag, text = '') {
        this.tagName = tag;
        this.attrs = new Map();
        this.textContent = text;
        this.children = [];
        this.hidden = false;
    }
    set innerHTML(_) { throw new Error('innerHTML is forbidden'); }
    getAttribute(name) { return this.attrs.has(name) ? this.attrs.get(name) : null; }
    setAttribute(name, value) { this.attrs.set(name, value); }
    removeAttribute(name) { this.attrs.delete(name); }
    querySelector(selector) { return this.querySelectorAll(selector)[0] ?? null; }
    querySelectorAll(selector) {
        const found = [];
        const visit = node => {
            for (const child of node.children) {
                if (child.tagName === '#text') continue;
                const isLink = selector === 'a[href]' && child.tagName === 'a' && child.getAttribute('href') !== null;
                const isHook = selector.startsWith('[data-page-') && child.getAttribute(selector.slice(1, -1)) !== null;
                if (isLink || isHook) found.push(child);
                visit(child);
            }
        };
        visit(this);
        return found;
    }
    replaceChildren(...children) { this.children = children; }
}
function anchor(href, text) {
    const node = new Element('a', text);
    node.setAttribute('href', href);
    return node;
}
const root = new Element('div');
const home = anchor('/moth/docs/', 'Documentation');
const links = new Element('nav');
const current = new Element('span');
home.setAttribute('data-page-home', '');
links.setAttribute('data-page-links', '');
current.setAttribute('data-page-current', '');
root.children = [home, links, current];
const outside = anchor('/elsewhere', 'Outside');
outside.setAttribute('aria-current', 'page');
let heading = new Element('h1', 'Heading');
globalThis.document = {
    baseURI: 'https://example.test/moth/docs/section/',
    title: 'Title fallback',
    getElementById: id => id === 'nav' ? root : null,
    querySelector: selector => selector === 'h1' ? heading : null,
    createElement: tag => new Element(tag),
    createTextNode: text => ({ tagName: '#text', textContent: text, children: [] }),
    addEventListener() { throw new Error('listener installed'); },
};
globalThis.window = {
    location: new URL('https://example.test/moth/docs/'),
    addEventListener() { throw new Error('listener installed'); },
};
function sync(path, expected) {
    window.location = new URL(path, 'https://example.test');
    const before = window.location.href;
    assert.deepEqual(page.syncNavigation('nav'), { ok: true, value: expected });
    assert.equal(window.location.href, before);
    assert.equal(outside.getAttribute('aria-current'), 'page');
}
function crumb() { return current.children.map(node => node.textContent).join(''); }
"#;

#[test]
fn navigation_tracks_routes_and_rebuilds_safe_breadcrumbs() {
    run_asset_in_node(
        "page",
        PAGE_SOURCE,
        &format!(
            "{DOM}{}",
            r#"
const packages = anchor('/moth/docs/packages/', 'Packages');
const deep = anchor('/moth/docs/packages/core/index.html?ignored#ignored', 'Core');
const duplicate = anchor('/moth/docs/packages/core/', 'Duplicate');
const leaf = anchor('/moth/docs/packages/core/time/', '<b>x</b>');
const near = anchor('/moth/docs/pack', 'Wrong');
const external = anchor('https://other.test/moth/docs/external-only/', 'External');
const malformed = anchor('http://[invalid', 'Malformed');
links.children = [deep, near, packages, duplicate, leaf, external, malformed];
current.hidden = true;
for (const path of ['/moth/docs/', '/moth/docs/index.html', '/moth/docs']) {
    sync(path, true);
    assert.equal(home.getAttribute('aria-current'), 'page');
    assert.equal(current.hidden, true);
    assert.equal(current.children.length, 0);
}
sync('/moth/docs/packages/core/time?x=1#y', true);
assert.equal(home.getAttribute('aria-current'), null);
assert.equal(leaf.getAttribute('aria-current'), 'page');
assert.equal(current.hidden, false);
assert.equal(crumb(), ' / Packages / Core / <b>x</b>');
assert.deepEqual(current.children.map(n => n.tagName), ['#text', 'a', '#text', 'a', '#text', 'span']);
assert.equal(current.children[1].getAttribute('href'), '/moth/docs/packages/');
sync('/moth/docs/packages/core/index.html', true);
assert.equal(leaf.getAttribute('aria-current'), null);
assert.equal(deep.getAttribute('aria-current'), 'page');
assert.equal(duplicate.getAttribute('aria-current'), null);
assert.equal(crumb(), ' / Packages / Core');
sync('/moth/docs/packages/core/unlisted', false);
assert.equal(deep.getAttribute('aria-current'), null);
assert.equal(crumb(), ' / Packages / Core / Heading');
sync('/moth/docs/external-only/', false);
assert.equal(external.getAttribute('aria-current'), null);
assert.equal(crumb(), ' / Heading');
heading = null;
sync('/moth/docs/packages/unknown', false);
assert.equal(crumb(), ' / Packages / Title fallback');
const encoded = anchor('/moth/docs/a%2Fb/', 'Encoded');
links.children.push(encoded);
sync('/moth/docs/a%2Fb/index.html', true);
sync('/moth/docs/a%2fb/', false);
sync('/moth/docs/a/b/', false);
sync('/moth/docs/A%2Fb/', false);
assert.equal(encoded.getAttribute('aria-current'), null);
const homeLabel = anchor('/moth/docs/same', 'Documentation');
const finalLabel = anchor('/moth/docs/same/child', 'Final');
const final = anchor('/moth/docs/same/child/end', 'Final');
links.children.push(homeLabel, finalLabel, final);
sync('/moth/docs/same/child/end', true);
assert.equal(crumb(), ' / Final');
home.setAttribute('href', '/');
const rootLink = anchor('/guide/', 'Guide');
links.children.push(rootLink);
sync('/guide/index.html', true);
assert.equal(crumb(), ' / Guide');
sync('/', true);
assert.equal(home.getAttribute('aria-current'), 'page');
assert.equal(current.hidden, true);
assert.equal(current.children.length, 0);
"#
        ),
    );
}

#[test]
fn navigation_reports_missing_root_and_hooks() {
    run_asset_in_node(
        "page",
        PAGE_SOURCE,
        &format!(
            "{DOM}{}",
            r#"
function failure(result, code, part) {
    assert.equal(result.ok, false);
    assert.equal(result.error.code, code);
    if (part) assert.ok(result.error.message.includes(part));
}
failure(page.syncNavigation(''), runtime.MOTH_ERROR_HOST_INVALID_ARGUMENT);
failure(page.syncNavigation('missing'), runtime.MOTH_ERROR_HOST_RESOURCE_NOT_FOUND);
for (const hook of ['data-page-home', 'data-page-links', 'data-page-current']) {
    const element = root.children.find(node => node.getAttribute(hook) !== null);
    element.removeAttribute(hook);
    failure(page.syncNavigation('nav'), runtime.MOTH_ERROR_HOST_RESOURCE_NOT_FOUND, hook);
    element.setAttribute(hook, '');
}
"#
        ),
    );
}

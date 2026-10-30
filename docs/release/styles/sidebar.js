// Desktop docs sidebar geometry. init_docs_sidebar() owns link marking, this only reads its result.
const desktop = matchMedia("(min-width: 961px)");
const navbar = document.querySelector('nav[aria-label="Primary"]');
const sidebar = document.getElementById("docs-sidebar");
const links = document.getElementById("docs-sidebar-links");
const toggle = document.getElementById("docs-sidebar-toggle");
let clearanceFrame = 0;

// The sidebar CSS derives its top from this, so it rises into the space the navbar scrolls out of.
function updateNavbarClearance() {
  clearanceFrame = 0;
  sidebar.style.setProperty("--docs-navbar-bottom", `${navbar.getBoundingClientRect().bottom}px`);
}

function scheduleNavbarClearance() {
  if (clearanceFrame === 0) clearanceFrame = requestAnimationFrame(updateNavbarClearance);
}

// Puts the current page's category heading at the top of the desktop link list's scrollport. Runs
// only on load and expansion so it never fights manual scrolling.
function alignCurrentCategory() {
  if (!desktop.matches || toggle.checked) return;
  let heading = links.querySelector('.docs-sidebar__link[aria-current="page"]');
  while (heading !== null && !heading.classList.contains("docs-sidebar__group")) {
    heading = heading.previousElementSibling;
  }
  if (heading === null) return;
  // Leave the heading's own margin visible so every category sits where the first one does at rest.
  const contentTop =
    links.getBoundingClientRect().top + links.clientTop + parseFloat(getComputedStyle(links).paddingTop);
  const headingTop = heading.getBoundingClientRect().top - parseFloat(getComputedStyle(heading).marginTop);
  links.scrollTop += headingTop - contentTop;
}

updateNavbarClearance();
addEventListener("scroll", scheduleNavbarClearance, { passive: true });
addEventListener("resize", scheduleNavbarClearance);
addEventListener("pageshow", scheduleNavbarClearance);
// Deferred classic scripts run before the page module, so wait for it to mark the current link.
addEventListener("DOMContentLoaded", alignCurrentCategory);
toggle.addEventListener("change", alignCurrentCategory);

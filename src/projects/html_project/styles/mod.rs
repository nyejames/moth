//! HTML-project-owned template style formatter modules.
//!
//! WHAT:
//! - Groups directive implementations that belong specifically to the HTML project builder.
//!
//! WHY:
//! - File ownership should make it obvious that `$html`, `$css`, `$code` and
//!   `$escape_html` are HTML-project directives even though the frontend
//!   executes their hooks.

pub(crate) mod code;
pub(crate) mod css;
pub(crate) mod escape_html;
pub(crate) mod html;
pub(crate) mod validation;

//! Frontend-owned template style formatter/validator modules.
//!
//! WHAT:
//! - Groups the compiler frontend's formatter implementations, including the `$html`, `$css`,
//!   `$escape_html` and `$code` formatters registered as built-in directives.
//!
//! WHY:
//! - The frontend owns every template formatter and makes them available under every builder.
//!   Keeping the implementations here separates formatter logic from generic template parsing
//!   while giving one owner for directive registration and execution.

// -------------------------
//  Public Modules
// -------------------------

pub(crate) mod code;
pub(crate) mod css;
pub(crate) mod escape_html;
pub(crate) mod html;
pub(crate) mod markdown;
pub(crate) mod raw;
pub(crate) mod validation;
pub(crate) mod whitespace;

#[cfg(test)]
mod tests;

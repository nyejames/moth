//! JS source text emission and indentation.
//!
//! WHAT: owns emitting formatted lines into the output buffer.
//! WHY: every JS backend emission path writes through here, so indentation lives in one place.
//!
//! This module must not own symbol lookup, block lookup, reachability, or
//! identifier generation. Those responsibilities belong to their focused owners.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_line(&mut self, line: &str) {
        if self.config.pretty {
            for _ in 0..self.indent {
                self.out.push_str("    ");
            }
        }

        self.out.push_str(line);
        self.out.push('\n');
    }

    pub(crate) fn emit_javascript_source(&mut self, source: &str) {
        for line in source.lines() {
            self.emit_line(line);
        }
    }

    pub(crate) fn with_indent<F>(&mut self, mut callback: F)
    where
        F: FnMut(&mut Self),
    {
        self.indent += 1;
        callback(self);
        self.indent -= 1;
    }
}

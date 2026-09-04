//! JS source text emission and indentation.
//!
//! WHAT: owns emitting formatted lines and source-location comments into the
//! output buffer.
//! WHY: every JS backend emission path writes through here, so indentation and
//! location tracking live in one place.
//!
//! This module must not own symbol lookup, block lookup, reachability, or
//! identifier generation. Those responsibilities belong to their focused owners.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

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

    pub(crate) fn emit_location_comment(&mut self, location: &SourceLocation) {
        if !self.config.emit_locations {
            return;
        }

        let line = location.start_pos.line_number + 1;
        let start = location.start_pos.char_column;
        let end = location.end_pos.char_column;
        self.emit_line(&format!("// source {line}:{start}-{end}"));
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

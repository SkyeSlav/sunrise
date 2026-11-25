//! Each stage shares macro definitions with its includes: the directives of a program are run
//! here, in the order the program reads them, and its code is expanded with the macros defined
//! so far, so the GLSL frontend gets text with no directive of its own left. [`Conditions`] holds
//! the macros; the nesting of `#if` ... `#endif` is [`crate::Conditionals`].
use super::{MAX_SCANNED, is_ident};
use crate::condition::{line_value, resolve_defined, value};
use crate::macros::{MAX_LINE_EXPANSION, Macros};

#[derive(Default)]
pub(super) struct Conditions {
    macros: Macros,
    scanned: usize,
}

impl Conditions {
    pub(super) fn new(defines: &[&str]) -> Self {
        let mut state = Self::default();
        for define in defines {
            let (name, value) = define.split_once(' ').unwrap_or((define, ""));
            state.macros.define(name, value);
        }
        state
    }

    pub(super) fn macros(&self) -> &Macros {
        &self.macros
    }

    pub(super) fn define(&mut self, name: &str, value: &str) {
        self.macros.define(name, value);
    }

    pub(super) fn define_line(&mut self, definition: &str) -> Result<(), String> {
        self.macros.define_line(definition)
    }

    pub(super) fn undef(&mut self, name: &str) {
        self.macros.undef(name);
    }

    /// `__LINE__` on the line being read: the line number GLSL gives it.
    pub(super) fn at_line(&mut self, line: u32) {
        self.macros.define("__LINE__", line.to_string());
    }

    pub(super) fn test(&mut self, kind: &str, rest: &str) -> Result<bool, String> {
        match kind {
            "ifdef" | "ifndef" => {
                if rest.is_empty() || !rest.chars().all(is_ident) {
                    return Err(format!("invalid #{kind} {rest}"));
                }
                Ok(self.macros.is_defined(rest) == (kind == "ifdef"))
            }
            _ => self.number(rest, false).map(|n| n != 0),
        }
    }

    /// The number of the line after a `#line` directive with this operand: GLSL numbers the
    /// directive's own line by it, as GL compilers do, so the next one is one more.
    pub(super) fn line(&mut self, rest: &str) -> Result<u32, String> {
        let number = self.number(rest, true)?;
        u32::try_from(number)
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| "line number outside GLSL range".into())
    }

    fn number(&mut self, expression: &str, line_directive: bool) -> Result<i64, String> {
        self.charge(expression.len())?;
        let resolved = resolve_defined(expression, |name| self.macros.is_defined(name))?;
        let expanded = self
            .macros
            .expand_within(&resolved, self.remaining().min(MAX_LINE_EXPANSION))
            .ok_or_else(|| super::ImportRefusal::TooMuchScanned.to_string())?;
        self.charge(expanded.len())?;
        if line_directive {
            line_value(&expanded)
        } else {
            value(&expanded)
        }
    }

    /// `code` with its macros expanded, a line of the program's own.
    pub(super) fn expand(&mut self, code: &str) -> Result<String, String> {
        let expanded = self
            .macros
            .expand_within(code, self.remaining().min(MAX_LINE_EXPANSION))
            .ok_or_else(|| super::ImportRefusal::TooMuchScanned.to_string())?;
        self.charge(expanded.len().saturating_sub(code.len()))?;
        Ok(expanded)
    }

    fn remaining(&self) -> usize {
        MAX_SCANNED.saturating_sub(self.scanned)
    }

    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.scanned += bytes;
        if self.scanned > MAX_SCANNED {
            return Err(super::ImportRefusal::TooMuchScanned.to_string());
        }
        Ok(())
    }
}

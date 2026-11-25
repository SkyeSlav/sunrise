//! The conditional directives of a pack's files (spec 1.5): the text after a line's `#`, and
//! how a condition that can't be evaluated is read. The arithmetic and the nesting are
//! `sunrise_glsl::eval_condition` and `sunrise_glsl::Conditionals`.

use crate::Macros;
use sunrise_glsl::{Conditionals, eval_condition};

/// The text after a line's `#`, split into the directive word and the rest without its `//`
/// comment. `None` when the first word is followed by something other than white space, `//` or
/// the end of the line (`#ifdefault=1` is a comment, not `#ifdef`).
pub(crate) fn split_directive(after_hash: &str) -> Option<(&str, &str)> {
    let line = after_hash.trim_start();
    let len = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let (word, rest) = line.split_at(len);
    let ok = rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with("//");
    ok.then(|| (word, rest.split("//").next().unwrap_or("").trim()))
}

/// Applies `#word rest` to `conditionals` when it is a conditional directive (`Ok(false)` when
/// it is not). A condition that can't be evaluated reads as false and returns why.
pub(crate) fn apply(
    conditionals: &mut Conditionals,
    word: &str,
    rest: &str,
    macros: &Macros,
) -> Result<bool, String> {
    conditionals.apply_with(word, |kind| test(kind, rest, macros))
}

fn test(word: &str, rest: &str, macros: &Macros) -> Result<bool, String> {
    let name = rest.split_whitespace().next().unwrap_or("");
    match word {
        "ifdef" => Ok(macros.is_defined(name)),
        "ifndef" => Ok(!macros.is_defined(name)),
        _ => eval_condition(rest, macros)
            .map(|v| v != 0)
            .map_err(|e| format!("`#{word} {rest}`: {e}; read as false")),
    }
}

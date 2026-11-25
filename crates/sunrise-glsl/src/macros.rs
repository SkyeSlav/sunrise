//! The macros a GLSL program defines and the preprocessor's expansion of them (GLSL 4.60 §3.3):
//! object-like and function-like macros, `##` pasting, and a macro not expanded again inside its
//! own expansion. The caller's own definitions (a pack's `MC_VERSION`, a pipeline's defines) are
//! made with [`Macros::define`]; the program's `#define` lines with [`Macros::define_line`].

use std::cell::Cell;
use std::collections::BTreeMap;

/// How many lines one macro call may span.
const MAX_CALL_LINES: usize = 64;

/// The most bytes of replacement text one expansion of a line or a condition may reach through
/// [`Macros::expand_within`]'s callers in this crate: a macro that doubles itself thirty times
/// is refused, not expanded.
pub(crate) const MAX_LINE_EXPANSION: usize = 1 << 16;

#[derive(Clone, Debug, PartialEq)]
struct Macro {
    /// `None` for an object-like macro; the parameter names of a function-like one.
    params: Option<Vec<String>>,
    body: String,
}

/// The macros defined so far, by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(BTreeMap<String, Macro>);

/// The bytes of replacement text an expansion may still add; none left stops it.
struct Budget {
    left: Cell<usize>,
    spent: Cell<bool>,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self {
            left: Cell::new(limit),
            spent: Cell::new(false),
        }
    }

    /// Takes `bytes`; `false` when there aren't that many, and the budget is spent from then on.
    fn take(&self, bytes: usize) -> bool {
        match self.left.get().checked_sub(bytes) {
            Some(left) => {
                self.left.set(left);
                true
            }
            None => {
                self.spent.set(true);
                false
            }
        }
    }
}

impl Macros {
    pub fn define(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let body = value.into();
        self.0.insert(name.into(), Macro { params: None, body });
    }

    /// The rest of a `#define` line: `NAME body`, or `NAME(a, b) body` (no space before the `(`)
    /// for a function-like macro.
    pub fn define_line(&mut self, rest: &str) -> Result<(), String> {
        let len = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
        let (name, tail) = rest.split_at(len);
        if name.is_empty() {
            return Err("`#define` without a name".into());
        }
        if let Some(params) = tail.strip_prefix('(') {
            let Some((params, body)) = params.split_once(')') else {
                return Err(format!("`#define {name}(` without `)`"));
            };
            let params = params
                .split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect();
            self.define_function(name, params, body.trim());
        } else {
            self.define(name, tail.trim());
        }
        Ok(())
    }

    pub fn define_function(&mut self, name: &str, params: Vec<String>, body: &str) {
        let body = body.to_owned();
        self.0.insert(
            name.to_owned(),
            Macro {
                params: Some(params),
                body,
            },
        );
    }

    pub fn undef(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The names defined, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// The value of an object-like macro.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .filter(|m| m.params.is_none())
            .map(|m| m.body.as_str())
    }

    /// Whether upcoming source lines continue a call, including one whose expanded name
    /// ends `text` and whose opening parenthesis is on the next nonempty line. A call whose
    /// expansion grows past [`MAX_LINE_EXPANSION`] is not taken to go on: its expansion refuses
    /// it where it is made.
    fn unfinished_call(&self, text: &str, upcoming: &[&str]) -> bool {
        let budget = Budget::new(MAX_LINE_EXPANSION);
        let (tokens, unfinished) = self.expand_tokens(expansion_tokens(text), &budget);
        !budget.spent.get()
            && (unfinished
                || (tokens.iter().rfind(|t| !t.blank()).is_some_and(|t| {
                    self.0.get_key_value(&t.text).is_some_and(|(name, m)| {
                        m.params.is_some() && !t.hidden.contains(&name.as_str())
                    })
                }) && upcoming
                    .iter()
                    .map(|line| line.trim_start())
                    .find(|line| !line.is_empty())
                    .is_some_and(|line| line.starts_with('('))))
    }

    /// `line` joined with the `following` lines its last macro call spans (an argument list that
    /// goes on past the line, up to [`MAX_CALL_LINES`] lines, none of them a directive), and how
    /// many of them it took; `None` when the call, if any, ends on `line`.
    pub fn join_call(&self, line: &str, following: &[&str]) -> Option<(String, usize)> {
        let mut joined: Option<String> = None;
        let mut taken = 0;
        while taken < following.len()
            && taken + 1 < MAX_CALL_LINES
            && !following[taken].trim_start().starts_with('#')
            && self.unfinished_call(joined.as_deref().unwrap_or(line), &following[taken..])
        {
            let joined = joined.get_or_insert_with(|| line.to_owned());
            joined.push(' ');
            joined.push_str(following[taken]);
            taken += 1;
        }
        joined.map(|joined| (joined, taken))
    }

    /// `text` with every macro expanded, repeatedly; a macro is not expanded again inside its own
    /// expansion, so `A` defined as `A + 1` stops. Function-like macros take their arguments
    /// (expanded first, raw beside `##`, which pastes) when a `(` follows their name.
    pub fn expand(&self, text: &str) -> String {
        let budget = Budget::new(usize::MAX);
        self.expand_tokens(expansion_tokens(text), &budget)
            .0
            .iter()
            .map(|t| t.text.as_str())
            .collect()
    }

    /// [`Macros::expand`], or `None` when the replacement text added grows past `limit` bytes.
    pub fn expand_within(&self, text: &str, limit: usize) -> Option<String> {
        let budget = Budget::new(limit);
        let tokens = self.expand_tokens(expansion_tokens(text), &budget).0;
        (!budget.spent.get()).then(|| tokens.iter().map(|t| t.text.as_str()).collect())
    }

    fn expand_tokens<'a>(
        &'a self,
        mut tokens: Vec<ExpansionToken<'a>>,
        budget: &Budget,
    ) -> (Vec<ExpansionToken<'a>>, bool) {
        let mut i = 0;
        let mut unfinished = false;
        while i < tokens.len() {
            let Some((name, m)) = self.0.get_key_value(tokens[i].text.as_str()) else {
                i += 1;
                continue;
            };
            if tokens[i].hidden.contains(&name.as_str()) {
                i += 1;
                continue;
            }
            let mut hidden = tokens[i].hidden.clone();
            let (mut body, next) = match &m.params {
                None => (expansion_tokens(&m.body), i + 1),
                Some(params) => {
                    let Some((args, next)) = arguments(&tokens, i + 1) else {
                        unfinished |= tokens[i + 1..]
                            .iter()
                            .find(|t| !t.blank())
                            .is_some_and(|t| t.text == "(");
                        i += 1;
                        continue;
                    };
                    // A call can cross a replacement boundary. Only disabling shared by
                    // its name and closing parenthesis carries into the new replacement.
                    hidden.retain(|h| tokens[next - 1].hidden.contains(h));
                    (self.substitute(params, &args, &m.body, budget), next)
                }
            };
            if !budget.take(body.iter().map(|t| t.text.len()).sum()) {
                return (tokens, unfinished);
            }
            hidden.push(name);
            for token in &mut body {
                for h in &hidden {
                    if !token.hidden.contains(h) {
                        token.hidden.push(h);
                    }
                }
            }
            // Rescan the replacement together with the remaining source. Its final name
            // may consume a call there; source tokens keep their own disabling context.
            tokens.splice(i..next, body);
        }
        (tokens, unfinished)
    }

    /// A function-like macro's body with its parameters replaced.
    fn substitute<'a>(
        &'a self,
        params: &[String],
        args: &[Vec<ExpansionToken<'a>>],
        body: &str,
        budget: &Budget,
    ) -> Vec<ExpansionToken<'a>> {
        let tokens = lex(body);
        let pasted = |at: usize, step: isize| {
            let mut j = at as isize + step;
            while (0..tokens.len() as isize).contains(&j) && tokens[j as usize].trim().is_empty() {
                j += step;
            }
            (0..tokens.len() as isize).contains(&j) && tokens[j as usize] == "##"
        };
        let mut out = Vec::new();
        for (at, token) in tokens.iter().enumerate() {
            match params.iter().position(|p| p == token) {
                Some(k) => {
                    let arg = args.get(k).map_or(&[][..], |a| a.as_slice());
                    let from = arg.iter().position(|t| !t.blank()).unwrap_or(arg.len());
                    let end = arg.iter().rposition(|t| !t.blank()).map_or(from, |n| n + 1);
                    let arg = arg[from..end].to_vec();
                    if pasted(at, -1) || pasted(at, 1) {
                        if arg.is_empty() {
                            // An empty paste operand must not capture the preceding token.
                            out.push(ExpansionToken::new(""));
                        } else {
                            out.extend(arg);
                        }
                    } else {
                        out.extend(self.expand_tokens(arg, budget).0);
                    }
                }
                None => out.push(ExpansionToken::new(token)),
            }
        }
        // `a ## b` → `ab`.
        let mut joined: Vec<ExpansionToken<'a>> = Vec::new();
        let mut paste = false;
        for token in out {
            if token.text == "##" {
                while joined.last().is_some_and(|t| t.blank()) {
                    joined.pop();
                }
                paste = true;
            } else if paste && token.blank() {
                continue;
            } else if paste {
                if let Some(mut left) = joined.pop() {
                    // An empty operand leaves the other token, including its hiding, intact.
                    if left.text.is_empty() {
                        left = token;
                    } else if !token.text.is_empty() {
                        left.text.push_str(&token.text);
                        left.hidden.retain(|h| token.hidden.contains(h));
                    }
                    joined.push(left);
                } else {
                    joined.push(token);
                }
                paste = false;
            } else {
                joined.push(token);
            }
        }
        joined.retain(|t| !t.text.is_empty());
        // A paste that makes no single token (`b ## (`, Alpha Piscium's
        // `colors2_colorspaces_ ## a ## _to_ ## b ##(x)`) stays the tokens it is made of, as
        // GL drivers' preprocessors leave it: the name before the `(` is then rescanned and can
        // call a macro.
        joined
            .into_iter()
            .flat_map(|token| {
                let parts = lex(&token.text);
                if parts.len() < 2 {
                    return vec![token];
                }
                parts
                    .into_iter()
                    .map(|text| ExpansionToken {
                        text: text.into(),
                        hidden: token.hidden.clone(),
                    })
                    .collect()
            })
            .collect()
    }
}

/// Disabling belongs to each token: rescanning a boundary must neither revive a recursive
/// replacement token nor hide an independent invocation in the surrounding source.
#[derive(Clone)]
struct ExpansionToken<'a> {
    text: String,
    hidden: Vec<&'a str>,
}

impl ExpansionToken<'_> {
    fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            hidden: Vec::new(),
        }
    }

    fn blank(&self) -> bool {
        !self.text.is_empty() && self.text.trim().is_empty()
    }
}

fn expansion_tokens(text: &str) -> Vec<ExpansionToken<'_>> {
    lex(text).into_iter().map(ExpansionToken::new).collect()
}

/// The arguments of a call whose `(` is the next non-blank token from `i`, split at top-level
/// commas, and the index after its `)`. `None` when no `(` follows or it never closes.
fn arguments<'a>(
    tokens: &[ExpansionToken<'a>],
    mut i: usize,
) -> Option<(Vec<Vec<ExpansionToken<'a>>>, usize)> {
    while tokens.get(i)?.blank() {
        i += 1;
    }
    if tokens[i].text != "(" {
        return None;
    }
    let (mut depth, mut args, mut from) = (0, Vec::new(), i + 1);
    for (j, t) in tokens.iter().enumerate().skip(i + 1) {
        match t.text.as_str() {
            "(" => depth += 1,
            ")" if depth == 0 => {
                args.push(tokens[from..j].to_vec());
                return Some((args, j + 1));
            }
            ")" => depth -= 1,
            "," if depth == 0 => {
                args.push(tokens[from..j].to_vec());
                from = j + 1;
            }
            _ => {}
        }
    }
    None
}

/// Preprocessing tokens: identifiers, numbers (with their suffixes and exponents), runs of
/// white space, `##`, and single characters. Concatenated they give back `text`.
pub fn lex(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let start = i;
        i += 1;
        if is_ident_start(c) {
            while i < bytes.len() && is_ident(bytes[i] as char) {
                i += 1;
            }
        } else if c.is_ascii_digit() || (c == '.' && bytes.get(i).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() {
                let d = bytes[i] as char;
                let exp_sign = (d == '+' || d == '-') && matches!(bytes[i - 1], b'e' | b'E');
                if !(is_ident(d) || d == '.' || exp_sign) {
                    break;
                }
                i += 1;
            }
        } else if c.is_whitespace() {
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
        } else if c == '#' && bytes.get(i) == Some(&b'#') {
            i += 1;
        } else {
            while !text.is_char_boundary(i) {
                i += 1;
            }
        }
        out.push(&text[start..i]);
    }
    out
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub(crate) fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_replaces_whole_words_and_stops_on_recursion() {
        let mut m = Macros::default();
        m.define("RES", "SIZE");
        m.define("SIZE", "512");
        m.define("LOOP", "LOOP + 1");
        assert_eq!(m.expand("RES RES 0.5"), "512 512 0.5");
        assert_eq!(
            m.expand("RESOLUTION colortex4 LOOP"),
            "RESOLUTION colortex4 LOOP + 1"
        );
        m.define("e5", "x");
        assert_eq!(m.expand("1e5"), "1e5");
    }

    #[test]
    fn function_like_macros_take_arguments_and_paste() {
        let mut m = Macros::default();
        m.define("N", "4");
        m.define_function("SQ", vec!["x".into()], "((x) * (x))");
        m.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
        m.define_function("PI", vec![], "3.14");
        assert_eq!(m.expand("SQ(N + 1)"), "((4 + 1) * (4 + 1))");
        assert_eq!(m.expand("SQ(min(N, 2))"), "((min(4, 2)) * (min(4, 2)))");
        assert_eq!(m.expand("CAT(N, 2) CAT(col, ortex4)"), "N2 colortex4");
        assert_eq!(m.expand("PI() SQ SQ(1"), "3.14 SQ SQ(1");
        assert_eq!(m.get("SQ"), None);
    }

    #[test]
    fn a_define_line_makes_either_kind_of_macro() {
        let mut m = Macros::default();
        m.define_line("N 4").unwrap();
        m.define_line("ADD(a, b) ((a) + (b))").unwrap();
        m.define_line("EMPTY").unwrap();
        assert_eq!(m.expand("ADD(N, 1) EMPTY."), "((4) + (1)) .");
        assert!(m.define_line("").is_err());
        assert!(m.define_line("BROKEN(a a").is_err());
        m.undef("N");
        assert!(!m.is_defined("N") && m.is_defined("ADD"));
        assert_eq!(m.names().collect::<Vec<_>>(), ["ADD", "EMPTY"]);
    }

    /// A call's arguments may go on over lines, and its name may stand alone before the `(`.
    #[test]
    fn a_call_is_joined_with_the_lines_its_arguments_span() {
        let mut m = Macros::default();
        m.define_function("CLAMP01", vec!["x".into()], "clamp(x, 0.0, 1.0)");
        assert_eq!(
            m.join_call("float v = CLAMP01(", &["a * (b", "+ c)", ");", "float w;"]),
            Some(("float v = CLAMP01( a * (b + c) );".to_owned(), 3))
        );
        assert_eq!(
            m.join_call("float v = CLAMP01", &["", "(a);"]).unwrap().1,
            2
        );
        assert_eq!(m.join_call("float v = CLAMP01(a);", &["float w;"]), None);
        assert_eq!(m.join_call("CLAMP01(", &["#define X", "a)"]), None);
    }

    /// Macros that double themselves are refused once their text grows past the limit, not
    /// expanded to the end.
    #[test]
    fn an_expansion_past_its_limit_is_refused() {
        let mut m = Macros::default();
        m.define("B0", "x");
        for n in 1..40 {
            m.define(format!("B{n}"), format!("B{} B{}", n - 1, n - 1));
        }
        assert_eq!(m.expand_within("B39", 1024), None);
        assert_eq!(
            m.expand_within("B3", 1024).as_deref(),
            Some("x x x x x x x x")
        );
        assert!(!m.unfinished_call("B39", &["(a)"]));
    }
}

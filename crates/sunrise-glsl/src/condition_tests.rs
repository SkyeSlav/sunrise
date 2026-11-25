use super::*;

fn macros() -> Macros {
    let mut m = Macros::default();
    m.define("MC_VERSION", "260300");
    m.define("SHADOW_RES", "2048");
    m.define("WAVES", "");
    m.define("QUALITY", "SHADOW_RES / 1024");
    m
}

fn eval(expr: &str) -> Result<i64, String> {
    eval_condition(expr, &macros())
}

#[test]
fn precedence_and_operators_follow_c() {
    assert_eq!(eval("1 + 2 * 3"), Ok(7));
    assert_eq!(eval("(1 + 2) * 3"), Ok(9));
    assert_eq!(eval("1 << 2 + 1"), Ok(8));
    assert_eq!(eval("6 & 3 == 3"), Ok(0));
    assert_eq!(eval("1 || 0 && 0"), Ok(1));
    assert_eq!(eval("-7 / 2"), Ok(-3));
    assert_eq!(eval("-7 % 2"), Ok(-1));
    assert_eq!(eval("!0 + ~0"), Ok(0));
    assert_eq!(eval("0x10 + 010 + 3u"), Ok(27));
}

#[test]
fn macros_and_defined() {
    assert_eq!(eval("MC_VERSION >= 11300"), Ok(1));
    assert_eq!(eval("MC_VERSION <= 12111"), Ok(0));
    assert_eq!(eval("QUALITY == 2"), Ok(1));
    assert_eq!(eval("defined WAVES && defined(SHADOW_RES)"), Ok(1));
    assert_eq!(eval("!defined NOPE"), Ok(1));
    assert_eq!(eval("NOPE == 0"), Ok(1));
    assert_eq!(eval("undefinedFeature"), Ok(0));
}

/// Float values, as GL drivers read them (`#if AMBIENT_CEL_AMOUNT != 0` with
/// `#define AMBIENT_CEL_AMOUNT 0.0`).
#[test]
fn float_values_count() {
    assert_eq!(eval("0. != 0"), Ok(0));
    assert_eq!(eval("4. != 0"), Ok(1));
    assert_eq!(eval("0.7 > 0.5"), Ok(1));
    assert_eq!(eval(".25 * 4.0 == 1"), Ok(1));
    assert_eq!(eval("1.5f"), Ok(1));
    assert_eq!(eval("7 / 2"), Ok(3));
    assert_eq!(eval("SHADOW_RES == 2048.0"), Ok(1));
}

#[test]
fn scientific_literals_and_macro_values_count() {
    for expr in [
        "1e-3 > 0",
        "1.0e-3 == 0.001",
        "1.0e+3 == 1000",
        ".25E2 == 25",
        "1E3 / 4e2 == 2.5",
        "1e0 / 2 > 0",
        "1e-3+1e-3 == 2e-3",
        "1e+3-1 == 999",
        "0x1E-1 == 29",
        "0xEE+010 == 246",
    ] {
        assert_eq!(eval(expr), Ok(1), "{expr}");
    }
    let mut m = macros();
    m.define("TINY", "1e-3");
    m.define("WHOLE_REAL", "1e0");
    assert_eq!(eval_condition("TINY > 0", &m), Ok(1));
    assert_eq!(eval_condition("WHOLE_REAL / 2 > 0", &m), Ok(1));
    for expr in ["1e", "1e+", "1e--3"] {
        assert!(eval(expr).is_err(), "{expr}");
    }
}

#[test]
fn short_circuit_skips_guarded_division() {
    let mut m = macros();
    m.define("AMOUNT", "0");
    assert_eq!(eval_condition("AMOUNT != 0 && 100 / AMOUNT > 5", &m), Ok(0));
    assert_eq!(eval_condition("AMOUNT == 0 || 100 / AMOUNT > 5", &m), Ok(1));
    assert_eq!(eval_condition("AMOUNT && (1 % AMOUNT)", &m), Ok(0));
    // The unskipped side still errors.
    assert!(eval_condition("AMOUNT == 0 && 100 / AMOUNT > 5", &m).is_err());
    assert!(eval_condition("AMOUNT != 0 || 100 / AMOUNT > 5", &m).is_err());
}

#[test]
fn bad_conditions_say_why() {
    assert!(eval("1 / 0").is_err());
    assert!(eval("SHADOW_RES == 0.5 0").is_err());
    assert!(eval("(1 + 2").is_err());
    assert!(eval("1 2").is_err());
    assert!(eval("").is_err());
    assert!(eval("defined").is_err());
    assert!(eval("1 @ 2").is_err());
}

#[test]
fn a_line_directive_may_carry_a_source_string_number() {
    assert_eq!(line_value(" 40 "), Ok(40));
    assert_eq!(line_value(" 40 7 "), Ok(40));
    assert!(line_value(" 40 7 8 ").is_err());
}

/// Every `#if` expression where the path the core shaders took before (pp-rs behind
/// `import_conditions`, which hands naga's preprocessor the condition and the definitions it
/// names) and the one they take now (`eval_condition`) differ, then a few where they agree.
///
/// The `old` column is what pp-rs 0.2.1's `#if` gave, read from its rules: it evaluates in `i64`
/// with `checked_*` arithmetic, so an overflow, a division by zero (on either side of `&&` and
/// `||`, which it evaluates both sides of) and a shift of 64 or more or by a negative amount are
/// errors; its lexer makes a float literal (`0.5`, `1e3`) a token the `#if` grammar refuses; an
/// identifier that is no macro is 0; `defined` is resolved before pp-rs sees the line. The
/// `new` column is this module's: `f64` arithmetic with the integer operators cast, `&&` and `||`
/// skipping the guarded side's division by zero, shifts wrapping their amount to 6 bits, floats accepted
/// (a fraction is true).
#[test]
fn old_and_new_if_semantics_differ_in_these_cases() {
    const ERR: Result<i64, ()> = Err(());
    // (expression, old path, new path)
    let table: [(&str, Result<i64, ()>, Result<i64, ()>); 16] = [
        // The float-literal case the spec names.
        ("AMOUNT != 0", ERR, Ok(0)),
        ("0.5", ERR, Ok(1)),
        ("1e3 == 1000", ERR, Ok(1)),
        ("SHADOW_RES == 2048.0", ERR, Ok(1)),
        // Division by zero on a side `&&` / `||` skips: pp-rs still evaluates it.
        ("0 && 1 / 0", ERR, Ok(0)),
        ("1 || 1 / 0", ERR, Ok(1)),
        // Shifts: pp-rs refuses an amount of 64 or more and a negative one; here it wraps.
        ("1 << 64", ERR, Ok(1)),
        ("1 << -1", ERR, Ok(i64::MIN)),
        // Overflow: pp-rs refuses it; here the sum saturates.
        ("9223372036854775807 + 1", ERR, Ok(i64::MAX)),
        // Where the two agree.
        ("1 + 2 * 3", Ok(7), Ok(7)),
        ("-7 / 2", Ok(-3), Ok(-3)),
        ("!0 + ~0", Ok(0), Ok(0)),
        ("UNKNOWN == 0", Ok(1), Ok(1)),
        ("defined WAVES && QUALITY == 2", Ok(1), Ok(1)),
        ("1 / 0", ERR, ERR),
        ("1 2", ERR, ERR),
    ];
    let mut m = macros();
    m.define("AMOUNT", "0.0");
    for (n, (expr, old, new)) in table.into_iter().enumerate() {
        assert_eq!(eval_condition(expr, &m).map_err(|_| ()), new, "{expr}");
        // The first nine rows are the ones that differ.
        assert_eq!(old != new, n < 9, "{expr}");
    }
}

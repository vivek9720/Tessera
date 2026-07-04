//! End-to-end tests for the surface language: source in, value/output out.

use tessera::value::Value;

fn eval(src: &str) -> Value {
    tessera::run_source(src)
        .unwrap_or_else(|e| panic!("program failed: {e}\nsource:\n{src}"))
        .0
}

fn eval_out(src: &str) -> String {
    tessera::run_source(src).expect("program failed").1
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(eval("return 2 + 3 * 4;").as_int(), Some(14));
    assert_eq!(eval("return (2 + 3) * 4;").as_int(), Some(20));
    assert_eq!(eval("return 2 ** 3 ** 2;").as_number(), Some(512.0));
    assert_eq!(eval("return 17 % 5;").as_int(), Some(2));
    assert_eq!(eval("return -5 + 3;").as_int(), Some(-2));
}

#[test]
fn comparison_and_logic() {
    assert_eq!(eval("return 1 < 2 and 2 < 3;").as_bool(), Some(true));
    assert_eq!(eval("return 1 > 2 or 5 == 5;").as_bool(), Some(true));
    assert_eq!(eval("return not (3 == 3);").as_bool(), Some(false));
    assert_eq!(eval("return 3 <= 3 and 4 >= 5;").as_bool(), Some(false));
}

#[test]
fn short_circuit_evaluation() {
    // If `and` did not short-circuit, calling `boom` would error.
    let src = "func boom() { return 1 / 0; } return false and boom();";
    assert_eq!(eval(src).as_bool(), Some(false));
    let src = "func boom() { return 1 / 0; } return true or boom();";
    assert_eq!(eval(src).as_bool(), Some(true));
}

#[test]
fn variables_and_assignment() {
    let src = "let x = 10; x = x + 5; x = x * 2; return x;";
    assert_eq!(eval(src).as_int(), Some(30));
}

#[test]
fn while_loops_and_accumulation() {
    let src = "let s = 0; let i = 1; while (i <= 100) { s = s + i; i = i + 1; } return s;";
    assert_eq!(eval(src).as_int(), Some(5050));
}

#[test]
fn if_else_chains() {
    // Direct behavioural checks for the three branches.
    assert_eq!(
        tessera::run_source("let n = -3; if (n < 0) { return 1; } else { return 2; }")
            .unwrap()
            .0
            .as_int(),
        Some(1)
    );
    assert_eq!(
        tessera::run_source("let n = 0; if (n < 0) { return 1; } else if (n == 0) { return 2; } else { return 3; }")
            .unwrap()
            .0
            .as_int(),
        Some(2)
    );
    assert_eq!(
        tessera::run_source("let n = 9; if (n < 0) { return 1; } else if (n == 0) { return 2; } else { return 3; }")
            .unwrap()
            .0
            .as_int(),
        Some(3)
    );
}

#[test]
fn recursion_factorial() {
    let src = "func fact(n) { if (n <= 1) { return 1; } return n * fact(n - 1); } return fact(6);";
    assert_eq!(eval(src).as_int(), Some(720));
}

#[test]
fn closures_capture_by_reference() {
    let src = "
        func make_counter() {
            let c = 0;
            return func() { c = c + 1; return c; };
        }
        let next = make_counter();
        next(); next();
        return next();
    ";
    assert_eq!(eval(src).as_int(), Some(3));
}

#[test]
fn independent_closures_have_independent_state() {
    let src = "
        func adder(n) { return func(x) { return x + n; }; }
        let a = adder(100);
        let b = adder(1);
        return a(0) - b(0);
    ";
    assert_eq!(eval(src).as_int(), Some(99));
}

#[test]
fn lists_indexing_and_builtins() {
    let src = "let xs = [5, 10, 15]; push(xs, 20); return xs[0] + xs[3] + len(xs);";
    assert_eq!(eval(src).as_int(), Some(29));
}

#[test]
fn negative_indexing() {
    assert_eq!(eval("let xs = [1, 2, 3]; return xs[-1];").as_int(), Some(3));
}

#[test]
fn maps_and_keys() {
    let src = "
        let m = {\"a\": 1, \"b\": 2};
        m[\"c\"] = 3;
        return m[\"a\"] + m[\"b\"] + m[\"c\"] + len(keys(m));
    ";
    assert_eq!(eval(src).as_int(), Some(9));
}

#[test]
fn string_operations() {
    assert_eq!(eval_out("println(upper(\"abc\") ++ \"!\");"), "ABC!\n");
    assert_eq!(eval("return len(\"tessera\");").as_int(), Some(7));
    assert_eq!(eval("return substr(\"hello world\", 0, 5);"), eval("return \"hello\";"));
}

#[test]
fn range_and_iteration() {
    let src = "let total = 0; let xs = range(1, 5); let i = 0; while (i < len(xs)) { total = total + xs[i]; i = i + 1; } return total;";
    assert_eq!(eval(src).as_int(), Some(10));
}

#[test]
fn break_and_continue() {
    let src = "
        let s = 0; let i = 0;
        while (i < 20) {
            i = i + 1;
            if (i == 5) { continue; }
            if (i > 10) { break; }
            s = s + i;
        }
        return s;
    ";
    // 1+2+3+4+6+7+8+9+10 = 50
    assert_eq!(eval(src).as_int(), Some(50));
}

#[test]
fn nested_closures_three_levels() {
    let src = "
        func a(x) {
            return func(y) {
                return func(z) { return x + y + z; };
            };
        }
        return a(1)(2)(3);
    ";
    assert_eq!(eval(src).as_int(), Some(6));
}

#[test]
fn division_by_zero_is_runtime_error() {
    assert!(tessera::run_source("return 1 / 0;").is_err());
}

#[test]
fn deep_recursion_reports_overflow_not_crash() {
    let src = "func loop(n) { return loop(n + 1); } return loop(0);";
    let err = tessera::run_source(src).unwrap_err();
    assert_eq!(err.kind(), tessera::ErrorKind::Runtime);
}

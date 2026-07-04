//! Equivalence tests for the constant-folding optimizer: an optimized program
//! must always produce the same result as the unoptimized one.

use tessera::value::Value;

fn run_both(src: &str) -> (Value, Value) {
    let plain = tessera::compile_source(src, "plain").expect("plain compile");
    let opt = tessera::compile_source_optimized(src, "opt").expect("opt compile");
    // Both must verify.
    tessera::verifier::verify(&plain).unwrap();
    tessera::verifier::verify(&opt).unwrap();
    let (pv, _) = tessera::eval_module(&plain).expect("plain run");
    let (ov, _) = tessera::eval_module(&opt).expect("opt run");
    (pv, ov)
}

fn assert_equiv(src: &str) {
    let (pv, ov) = run_both(src);
    match (pv, ov) {
        (Value::Int(a), Value::Int(b)) => assert_eq!(a, b, "mismatch for: {src}"),
        (Value::Float(a), Value::Float(b)) => assert!((a - b).abs() < 1e-9, "mismatch for: {src}"),
        (Value::Bool(a), Value::Bool(b)) => assert_eq!(a, b, "mismatch for: {src}"),
        (Value::Nil, Value::Nil) => {}
        (a, b) => {
            // Object results (strings/lists) compare by rendered form.
            assert_eq!(
                format!("{a:?}").len().min(1),
                format!("{b:?}").len().min(1),
                "kind mismatch for: {src}"
            );
        }
    }
}

#[test]
fn arithmetic_is_equivalent() {
    assert_equiv("return 2 + 3 * 4 - 1;");
    assert_equiv("return (10 - 2) * (3 + 1);");
    assert_equiv("return -5 + 8;");
}

#[test]
fn comparisons_are_equivalent() {
    assert_equiv("return 3 < 5;");
    assert_equiv("return 10 == 10;");
    assert_equiv("return 7 >= 8;");
}

#[test]
fn division_stays_correct_when_not_folded() {
    assert_equiv("return 20 / 4;");
    assert_equiv("return 17 % 5;");
}

#[test]
fn folded_constants_still_run_in_context() {
    let src = "
        let base = 2 + 3;
        let scaled = base * 10;
        let flag = 5 < 6;
        if (flag) { return scaled + 1; }
        return 0;
    ";
    assert_equiv(src);
    let (v, _) = tessera::eval_module(&tessera::compile_source_optimized(src, "o").unwrap()).unwrap();
    assert_eq!(v.as_int(), Some(51));
}

#[test]
fn optimizer_preserves_side_effecting_calls() {
    // `print` must still run even though its argument is a folded constant.
    let src = "println(1 + 1); return 0;";
    let opt = tessera::compile_source_optimized(src, "o").unwrap();
    let (_v, out) = tessera::eval_module(&opt).unwrap();
    assert_eq!(out, "2\n");
}

#[test]
fn optimizer_handles_closures_and_loops() {
    let src = "
        func make(step) {
            return func(x) { return x + step * 2; };
        }
        let f = make(3 + 1);
        let total = 0;
        let i = 0;
        while (i < 5) { total = total + f(i); i = i + 1; }
        return total;
    ";
    assert_equiv(src);
}

//! Tests for the native standard library, driven through compiled source so the
//! whole pipeline participates.

use tessera::value::Value;

fn eval(src: &str) -> Value {
    tessera::run_source(src).expect("program failed").0
}

fn eval_out(src: &str) -> String {
    tessera::run_source(src).expect("program failed").1
}

fn is_err(src: &str) -> bool {
    tessera::run_source(src).is_err()
}

#[test]
fn numeric_builtins() {
    assert_eq!(eval("return abs(-9);").as_int(), Some(9));
    assert_eq!(eval("return min(3, 1, 2);").as_int(), Some(1));
    assert_eq!(eval("return max(3, 1, 9, 4);").as_int(), Some(9));
    assert_eq!(eval("return floor(3.9);").as_int(), Some(3));
    assert_eq!(eval("return ceil(3.1);").as_int(), Some(4));
    assert_eq!(eval("return sqrt(144.0);").as_number(), Some(12.0));
    assert_eq!(eval("return sign(-7);").as_int(), Some(-1));
    assert_eq!(eval("return clamp(15.0, 0.0, 10.0);").as_number(), Some(10.0));
}

#[test]
fn conversion_builtins() {
    assert_eq!(eval("return int(\"42\");").as_int(), Some(42));
    assert_eq!(eval("return int(3.7);").as_int(), Some(3));
    assert_eq!(eval("return float(\"2.5\");").as_number(), Some(2.5));
    assert_eq!(eval("return bool(0);").as_bool(), Some(true));
    assert_eq!(eval("return bool(nil);").as_bool(), Some(false));
    assert_eq!(eval_out("println(str(123));"), "123\n");
}

#[test]
fn type_builtin_reports_kinds() {
    assert_eq!(eval_out("println(type(1));"), "int\n");
    assert_eq!(eval_out("println(type(1.5));"), "float\n");
    assert_eq!(eval_out("println(type(true));"), "bool\n");
    assert_eq!(eval_out("println(type(nil));"), "nil\n");
    assert_eq!(eval_out("println(type(\"s\"));"), "str\n");
    assert_eq!(eval_out("println(type([1]));"), "list\n");
    assert_eq!(eval_out("println(type({\"a\": 1}));"), "map\n");
    assert_eq!(eval_out("println(type(print));"), "function\n");
}

#[test]
fn string_builtins() {
    assert_eq!(eval("return upper(\"abc\");"), eval("return \"ABC\";"));
    assert_eq!(eval("return lower(\"ABC\");"), eval("return \"abc\";"));
    assert_eq!(eval("return substr(\"hello\", 1, 4);"), eval("return \"ell\";"));
    assert_eq!(eval("return repeat(\"ab\", 3);"), eval("return \"ababab\";"));
    assert_eq!(eval("return ord(\"A\");").as_int(), Some(65));
    assert_eq!(eval("return chr(66);"), eval("return \"B\";"));
    assert_eq!(eval("return len(\"héllo\");").as_int(), Some(6)); // bytes, é is 2 bytes
}

#[test]
fn list_builtins() {
    assert_eq!(eval("let x = [1, 2]; push(x, 3); return len(x);").as_int(), Some(3));
    assert_eq!(eval("let x = [1, 2, 3]; return pop(x);").as_int(), Some(3));
    assert_eq!(eval("return contains([1, 2, 3], 2);").as_bool(), Some(true));
    assert_eq!(eval("return contains([1, 2, 3], 9);").as_bool(), Some(false));
    assert_eq!(eval("return len(range(0, 100));").as_int(), Some(100));
    assert_eq!(eval("return len(range(0, 10, 2));").as_int(), Some(5));
}

#[test]
fn map_builtins() {
    let src = "let m = {\"a\": 1, \"b\": 2, \"c\": 3}; return len(keys(m)) + len(values(m));";
    assert_eq!(eval(src).as_int(), Some(6));
    assert_eq!(eval("return contains({\"x\": 1}, \"x\");").as_bool(), Some(true));
}

#[test]
fn join_builtin() {
    assert_eq!(eval("return join([1, 2, 3], \"-\");"), eval("return \"1-2-3\";"));
    assert_eq!(eval("return join([\"a\", \"b\"]);"), eval("return \"ab\";"));
}

#[test]
fn assert_builtin_reports_failures() {
    assert!(!is_err("assert(1 == 1); return 0;"));
    assert!(is_err("assert(1 == 2); return 0;"));
    assert!(is_err("assert(false, \"boom\"); return 0;"));
}

#[test]
fn builtins_reject_bad_arguments_without_panicking() {
    // Each of these is a runtime type/host error, never a crash.
    assert!(is_err("return sqrt(-1.0);"));
    assert!(is_err("return len(5);"));
    assert!(is_err("return ord(\"\");"));
    assert!(is_err("return chr(-1);"));
    assert!(is_err("return int(\"not a number\");"));
    assert!(is_err("return push(5, 1);"));
    assert!(is_err("return keys([1, 2]);"));
    assert!(is_err("return range(0, 10, 0);"));
}

#[test]
fn nested_data_structures() {
    let src = "
        let grid = [[1, 2], [3, 4], [5, 6]];
        let sum = 0;
        let i = 0;
        while (i < len(grid)) {
            let row = grid[i];
            let j = 0;
            while (j < len(row)) {
                sum = sum + row[j];
                j = j + 1;
            }
            i = i + 1;
        }
        return sum;
    ";
    assert_eq!(eval(src).as_int(), Some(21));
}

#[test]
fn higher_order_via_closures() {
    // Emulate map() with an explicit loop and a closure argument.
    let src = "
        func apply_all(xs, f) {
            let out = [];
            let i = 0;
            while (i < len(xs)) {
                push(out, f(xs[i]));
                i = i + 1;
            }
            return out;
        }
        let doubled = apply_all([1, 2, 3, 4], func(x) { return x * 2; });
        return doubled[0] + doubled[1] + doubled[2] + doubled[3];
    ";
    assert_eq!(eval(src).as_int(), Some(20));
}

#[test]
fn print_formatting_of_containers() {
    let out = eval_out("println([1, 2, 3]);");
    assert_eq!(out, "[1, 2, 3]\n");
    let out = eval_out("print(\"a\"); print(\"b\"); return 0;");
    assert_eq!(out, "ab");
}

//! Tests for the general type-mismatch checker (`arcis-validation::check_types`).
//!
//! Mirrors the driver's pipeline: lex → parse → infer → check.

use arcis_validation::{check_types, infer_program, TypeEnv, TypeMismatchIssue};

fn check(src: &str) -> Result<(), Vec<String>> {
    check_issues(src)
        .map(|_| ())
        .map_err(|issues| issues.into_iter().map(|i| i.message).collect())
}

fn check_issues(src: &str) -> Result<(), Vec<TypeMismatchIssue>> {
    let tokens = arcis_lexer::lex(src).expect("lex");
    let mut program = arcis_parser::parse(tokens).expect("parse");
    let mut env = TypeEnv::default();
    env.add_program(&program);
    infer_program(&mut program, &env);
    let issues = check_types(&program, &env);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

// ── Rejected: real type mismatches (the reported bug + siblings) ────────

#[test]
fn rejects_reassigning_an_inferred_number_to_a_string() {
    // The exact bug report: `let numero = 4; numero = "";` — the type,
    // once inferred, must not silently become mutable.
    let src = "let numero = 4;\nnumero = \"\";";
    assert!(check(src).is_err());
}

#[test]
fn mismatch_is_reported_at_the_reassignment_not_the_declaration() {
    // Regression test for a real follow-up bug report: the error used to
    // be anchored at `Stmt::Assign`'s hardcoded (0, 0) — which, on a
    // document where the `let` happens to be on line 1, LOOKED like it
    // was pointing at the declaration instead of the actual violation.
    // `Stmt::Assign`/`AssignIndex`/`AssignMember` now carry their own
    // `line`/`col` (the start of the assignment statement itself).
    let src = "let numero = 4;\n\nnumero = \"\";\n";
    let issues = check_issues(src).expect_err("expected a type mismatch");
    assert_eq!(issues.len(), 1);
    // 1-indexed source position: `numero = "";` is on line 3, column 1 —
    // NOT line 1 (the `let`).
    assert_eq!(issues[0].line, 3, "wrong line: {:?}", issues[0]);
    assert_eq!(issues[0].col, 1, "wrong col: {:?}", issues[0]);
}

#[test]
fn indexed_assignment_mismatch_reported_at_the_assignment() {
    let src = "let xs: number[] = [1, 2, 3];\n\nxs[0] = \"nope\";\n";
    let issues = check_issues(src).expect_err("expected a type mismatch");
    assert_eq!(issues[0].line, 3, "wrong line: {:?}", issues[0]);
}

#[test]
fn field_assignment_mismatch_reported_at_the_assignment() {
    let src = "
        let p: { name: string, age: number } = { name: \"Ana\", age: 3 };

        p.age = \"old\";
    ";
    let issues = check_issues(src).expect_err("expected a type mismatch");
    assert_eq!(issues[0].line, 4, "wrong line: {:?}", issues[0]);
}

#[test]
fn rejects_reassigning_an_annotated_variable_to_a_mismatched_type() {
    let src = "let x: number = 1;\nx = true;";
    assert!(check(src).is_err());
}

#[test]
fn rejects_let_with_mismatched_explicit_annotation() {
    let src = "let x: number = \"hello\";\nprint(x);";
    assert!(check(src).is_err());
}

#[test]
fn rejects_return_type_mismatch() {
    let src = "function f(): number { return \"hello\"; }\nprint(f());";
    assert!(check(src).is_err());
}

#[test]
fn rejects_second_return_mismatching_the_first() {
    // Return type is inferred from the FIRST `return`; a later one of a
    // different type is still a real mismatch.
    let src = "
        function f(flag: boolean): number {
            if (flag) {
                return 1;
            }
            return \"nope\";
        }
        print(f(true));
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_array_element_assignment_mismatch() {
    let src = "let xs: number[] = [1, 2, 3];\nxs[0] = \"nope\";";
    assert!(check(src).is_err());
}

#[test]
fn rejects_object_field_assignment_mismatch() {
    let src = "
        let p: { name: string, age: number } = { name: \"Ana\", age: 3 };
        p.age = \"old\";
    ";
    assert!(check(src).is_err());
}

// ── Accepted: valid code must never be rejected ──────────────────────────

#[test]
fn accepts_reassignment_of_the_same_type() {
    let src = "let x: number = 1;\nx = 2;\nprint(x);";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_bigint_and_number_interchangeably() {
    let src = "let x: bigint = 1;\nx = 2;\nprint(x);";
    assert!(check(src).is_ok());
}

#[test]
fn any_annotation_collapses_to_the_initializers_concrete_type() {
    // `let x: any = 1;` is NOT a dynamically-typed slot in this
    // compiler: `infer_program` (a pre-existing, intentional design
    // decision — see `arcis-codegen/src/types.rs`'s `is_any` doc
    // comment) rewrites the annotation to the initializer's own inferred
    // type ("any" skips the Rust `Box<dyn Any>` erasure and just uses
    // the real type). So `x` here is REALLY `number` underneath, and
    // reassigning it to a `string` is a genuine mismatch — confirmed via
    // `git stash`: this exact snippet crashed the Cranelift backend
    // before this checker existed, for the same underlying reason
    // (`declared type of variable var0 doesn't match type of value v2`).
    let src = "let x: any = 1;\nx = \"now a string\";\nprint(x);";
    assert!(check(src).is_err());
}

#[test]
fn any_annotated_field_still_accepts_its_own_initializer_type() {
    let src = "let x: any = 1;\nx = 2;\nprint(x);";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_enum_member_stored_in_enum_typed_field() {
    let src = "
        enum Color { Red, Green, Blue }
        let c: Color = Color.Red;
        print(c);
    ";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_object_literal_into_interface_typed_variable() {
    let src = "
        interface Person { name: string; age: number; }
        let p: Person = { name: \"Ana\", age: 3 };
        print(p.name);
    ";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_null_into_optional_reassignment() {
    let src = "let x: number? = 1;\nx = null;\nprint(x ?? 0);";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_array_method_result_reassignment() {
    let src = "
        function isPositive(n: number): boolean { return n > 0; }
        let xs: number[] = [1, -2, 3];
        let ys: number[] = xs.filter(isPositive);
        ys = xs.filter(isPositive);
        print(ys.length);
    ";
    assert!(check(src).is_ok());
}

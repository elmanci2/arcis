//! Tests for the null-safety checker (`arcis-validation::check_null_safety`)
//! and the control-flow narrowing rewrite it depends on
//! (`arcis-validation::narrow_program`).
//!
//! Mirrors the pipeline `arcis-driver` runs: lex → parse → infer → narrow →
//! check.

use arcis_validation::{check_null_safety, infer_program, narrow_program, TypeEnv};

/// `Ok(())` when the program is sound; `Err(messages)` otherwise.
fn check(src: &str) -> Result<(), Vec<String>> {
    let tokens = arcis_lexer::lex(src).expect("lex");
    let mut program = arcis_parser::parse(tokens).expect("parse");
    let mut env = TypeEnv::default();
    env.add_program(&program);
    infer_program(&mut program, &env);
    narrow_program(&mut program);
    let issues = check_null_safety(&program, &env);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues.into_iter().map(|i| i.message).collect())
    }
}

// ── Rejected: unresolved optional flows ─────────────────────────────────

#[test]
fn rejects_optional_assigned_to_non_optional_let() {
    let src = "
        function find(): number? { return null; }
        let x: number = find();
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_null_literal_assigned_to_non_optional_let() {
    let src = "let x: number = null;";
    assert!(check(src).is_err());
}

#[test]
fn rejects_double_optional_fallback() {
    let src = "
        function find(): number? { return null; }
        let x: number = find() ?? find();
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_member_access_on_optional_receiver() {
    let src = "
        interface Product { name: string; }
        function find(): Product? { return null; }
        let n: string = find().name;
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_optional_field_used_directly() {
    // Inline object type, not a named `interface` — resolving an
    // `interface`/`type` name to its shape is `arcis-codegen`'s job
    // (`resolve_program_types`, run by the driver before inference); this
    // crate's own pipeline only guarantees `Type::Object` shapes that are
    // ALREADY concrete, which an inline annotation is by construction.
    let src = "
        let p: { name: string, nickname?: string } = { name: \"Ana\" };
        print(p.nickname.length);
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_optional_argument_to_non_optional_param() {
    let src = "
        function greet(name: string): string { return \"hi \" + name; }
        function maybeName(): string? { return null; }
        let g: string = greet(maybeName());
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_optional_arithmetic_operand() {
    let src = "
        function find(): number? { return null; }
        let x: number = find() + 1;
    ";
    assert!(check(src).is_err());
}

#[test]
fn rejects_optional_returned_from_non_optional_function() {
    let src = "
        function find(): number? { return null; }
        function get(): number {
            return find();
        }
    ";
    assert!(check(src).is_err());
}

// ── Accepted: every resolution strategy the language offers ────────────

#[test]
fn accepts_nullish_coalesce_with_solid_fallback() {
    let src = "
        function find(): number? { return null; }
        let x: number = find() ?? 0;
    ";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_narrowed_if_not_null() {
    let src = "
        function find(): number? { return 5; }
        let m: number? = find();
        if (m != null) {
            print(m + 1);
        }
    ";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_guard_clause_early_return() {
    let src = "
        function find(): number? { return 5; }
        function show(): void {
            let m: number? = find();
            if (m == null) {
                return;
            }
            print(m + 1);
        }
    ";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_explicit_non_null_assertion() {
    let src = "
        function find(): number? { return 5; }
        let x: number = find()!;
    ";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_optional_target_receiving_null() {
    let src = "let x: number? = null;";
    assert!(check(src).is_ok());
}

#[test]
fn accepts_unannotated_binding_of_an_optional_value() {
    // No explicit annotation: inference marks `x` itself as optional, so
    // there is nothing to flag HERE — any later unsafe USE of `x` is what
    // gets checked (see the rejection tests above).
    let src = "
        function find(): number? { return null; }
        let x = find();
    ";
    assert!(check(src).is_ok());
}

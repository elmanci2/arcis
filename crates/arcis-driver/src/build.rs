//! Build pipeline: link → validate → codegen → write to disk.
//!
//! Two output layouts are supported for the Rust backend, decided by
//! whether the entry's directory has a `Cargo.toml` beside it:
//!
//! - **Direct rustc**: write `<id>.rs` files directly under `bin/`.
//! - **Cargo project**: write `bin/<pkg>/Cargo.toml` + `bin/<pkg>/src/<id>.rs`
//!   for use with `cargo build`.
//!
//! The Cranelift backend always writes object files directly to `bin/`
//! (one `.o` per module + `arcis_runtime.o` + `arcis_runtime.c`).

use std::fs;
use std::path::{Path, PathBuf};

use arcis_ast::{ArrayElement, ArrowBody, Expr, ExportDefault, Function, ObjectField, Program, Stmt, Type};
use arcis_linker::Module;

/// Run the full pipeline: resolve modules, validate them, generate sources
/// for the chosen backend, and write them to disk.
pub(crate) fn run(input: &Path, backend: super::Backend) -> Result<super::BuildOutput, String> {
    let mut modules = arcis_linker::resolve(input)?;

    // Alpha-rename shadowed bindings before anything else sees the AST, so
    // validation (which treats `if`/`while`/`for` bodies as sharing their
    // enclosing scope) never reports valid shadowing as a duplicate, and
    // codegen's flat name-keyed maps never have to reason about scope.
    for m in &mut modules {
        arcis_validation::resolve_shadowing(&mut m.program);
    }

    for m in &modules {
        let issues = arcis_validation::validate(&m.program);
        if !issues.is_empty() {
            return Err(arcis_validation::format_issues(
                &issues,
                &m.path.display().to_string(),
            ));
        }
    }

    // Resolve interfaces / type aliases / enum names in type position, then
    // run type inference to fill in missing annotations (let/const types,
    // for-of element types, function return types). Both backends receive
    // the same fully-annotated ASTs.
    let mut modules = arcis_codegen::resolve_program_types(&modules);
    let mut env = arcis_validation::TypeEnv::default();
    for m in &modules {
        env.add_program(&m.program);
    }
    for m in &mut modules {
        arcis_validation::infer_program(&mut m.program, &env);
    }

    // Null-safety: every optional (`T?`) value must be resolved (`??
    // fallback`, a null-check guard, or `!`) before it reaches a place that
    // expects a guaranteed value — enforced as a hard compile error, not a
    // lint. `narrow_program` rewrites recognized guards
    // (`if (x != null) { ... }`) into a freshly-named `let` plus a rename
    // of every subsequent reference in scope (see `narrow.rs`'s doc comment
    // for why it mints its own unique name instead of reusing
    // `resolve_shadowing`).
    for m in &mut modules {
        arcis_validation::narrow_program(&mut m.program);
    }
    let mut null_issues = Vec::new();
    for m in &modules {
        for issue in arcis_validation::check_null_safety(&m.program, &env) {
            null_issues.push(format!(
                "{}:{}:{}: {}",
                m.path.display(),
                issue.line,
                issue.col,
                issue.message
            ));
        }
    }
    if !null_issues.is_empty() {
        return Err(format!(
            "null-safety error{}:\n{}",
            if null_issues.len() == 1 { "" } else { "s" },
            null_issues.join("\n")
        ));
    }

    // General type-mismatch check: once a binding's type is established
    // (annotation or inference), assigning/returning/storing an
    // incompatible value is a compile error — on both backends,
    // identically. Without this, an incompatible reassignment either
    // produces a confusing `rustc` error (Rust backend) or crashes the
    // Cranelift backend outright with a raw verifier panic.
    let mut type_issues = Vec::new();
    for m in &modules {
        for issue in arcis_validation::check_types(&m.program, &env) {
            type_issues.push(format!(
                "{}:{}:{}: {}",
                m.path.display(),
                issue.line,
                issue.col,
                issue.message
            ));
        }
    }
    if !type_issues.is_empty() {
        return Err(format!(
            "type error{}:\n{}",
            if type_issues.len() == 1 { "" } else { "s" },
            type_issues.join("\n")
        ));
    }

    // `json(...)` calls: `infer.rs`'s type inference treats a bad path/
    // malformed JSON permissively (so the LSP stays resilient on every
    // keystroke) — this is the authoritative re-check that turns that into
    // a real compile error instead of a silently-broken binary.
    let mut json_issues = Vec::new();
    for m in &modules {
        for issue in arcis_validation::check_json_calls(&m.program) {
            json_issues.push(format!(
                "{}:{}:{}: {}",
                m.path.display(),
                issue.line,
                issue.col,
                issue.message
            ));
        }
    }
    if !json_issues.is_empty() {
        return Err(format!(
            "json error{}:\n{}",
            if json_issues.len() == 1 { "" } else { "s" },
            json_issues.join("\n")
        ));
    }

    // Generics are Rust-backend-only (see `check_no_generics`'s doc comment
    // for why this can't just be left to `arcis-codegen-cranelift`'s own
    // `Result` plumbing — some of its callers silently swallow `from_ast`
    // errors and would miscompile instead of rejecting).
    if backend == super::Backend::Cranelift {
        for m in &modules {
            check_no_generics(&m.program, &m.path.display().to_string())?;
        }
    }

    let bin_dir = PathBuf::from("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| format!("could not create `bin/`: {}", e))?;
    let root_id = modules[0].id.clone();

    match backend {
        super::Backend::Rust => {
            let generated = arcis_codegen::generate_all(&modules)?;
            let entry_dir = modules[0].path.parent().ok_or_else(|| {
                format!(
                    "could not determine the directory of `{}`",
                    modules[0].path.display()
                )
            })?;
            let user_cargo_toml = entry_dir.join("Cargo.toml");

            if user_cargo_toml.exists() {
                Ok(write_cargo_layout(
                    &modules,
                    &generated,
                    &bin_dir,
                    &user_cargo_toml,
                    &root_id,
                )?)
            } else {
                write_rustc_layout(&modules, &generated, &bin_dir, &root_id)
            }
        }
        super::Backend::Cranelift => {
            // Clean stale .o files from previous builds.
            if let Ok(entries) = fs::read_dir(&bin_dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().map_or(false, |x| x == "o") {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
            let triple = target_lexicon::Triple::host();
            arcis_codegen_cranelift::compile_to_object(&modules, &bin_dir, &triple)?;
            // Write the runtime source alongside so the linker step can
            // also use `cc` on it.
            let runtime_c_path = bin_dir.join("arcis_runtime.c");
            fs::write(
                &runtime_c_path,
                arcis_codegen_cranelift::runtime::RUNTIME_C_SOURCE,
            )
            .map_err(|e| {
                format!("could not write `{}`: {}", runtime_c_path.display(), e)
            })?;
            Ok(super::BuildOutput {
                sources: Vec::new(),
                root_id,
                cargo_dir: None,
                pkg_name: None,
                backend: super::Backend::Cranelift,
            })
        }
    }
}

/// Reject every generic construct (a `function`/`interface`/`type` alias
/// declared with `<T, ...>`, an explicit-turbofish call `f<T>(...)`, or a
/// `Type::Generic` usage anywhere in a type position) AND every `json(...)`
/// call, before the Cranelift backend ever runs. Both are Rust-backend-only
/// (`json(...)`'s runtime deserialization leans on `serde`, same as
/// generics leaning on real Rust generics); this is the ONE place both
/// guarantees are enforced.
///
/// Deliberately NOT delegated to `arcis-codegen-cranelift`'s own type
/// lowering (`types::from_ast`): several of its callers do
/// `.unwrap_or(ArcisType::Number)` on `from_ast`'s `Result`, silently
/// swallowing an `Err` and treating a generic param/return as if it were
/// `f64` — miscompiling instead of rejecting. Running this walk once, here,
/// before either backend's real work starts, is airtight regardless of what
/// any downstream code does with a `Result`.
fn check_no_generics(program: &Program, path: &str) -> Result<(), String> {
    for stmt in &program.stmts {
        check_stmt_no_generics(stmt, path)?;
    }
    Ok(())
}

fn generics_err(path: &str, kind: &str, name: &str) -> String {
    format!("{path}: generic {kind} `{name}` is not yet supported by the Cranelift backend — use --backend rust")
}

fn generic_usage_err(path: &str) -> String {
    format!("{path}: use of a generic type is not yet supported by the Cranelift backend — use --backend rust")
}

fn json_unsupported_err(path: &str) -> String {
    format!("{path}: json(...) is not supported by the Cranelift backend — use --backend rust")
}

fn check_ty_no_generics(ty: Option<&Type>, path: &str) -> Result<(), String> {
    match ty {
        Some(t) if type_contains_generic(t) => Err(generic_usage_err(path)),
        _ => Ok(()),
    }
}

fn type_contains_generic(ty: &Type) -> bool {
    match ty {
        Type::Generic { .. } => true,
        Type::Array(inner) | Type::Optional(inner) => type_contains_generic(inner),
        Type::Object { fields, .. } => fields.iter().any(|(_, t, _)| type_contains_generic(t)),
        Type::Union(members) | Type::Intersection(members) => members.iter().any(type_contains_generic),
        Type::Function { params, return_type } => {
            params.iter().any(type_contains_generic) || type_contains_generic(return_type)
        }
        _ => false,
    }
}

fn check_function_no_generics(f: &Function, path: &str) -> Result<(), String> {
    if !f.type_params.is_empty() {
        return Err(generics_err(path, "function", &f.name));
    }
    for p in &f.params {
        check_ty_no_generics(Some(&p.ty), path)?;
    }
    check_ty_no_generics(Some(&f.return_type), path)?;
    for s in &f.body {
        check_stmt_no_generics(s, path)?;
    }
    Ok(())
}

fn check_stmt_no_generics(stmt: &Stmt, path: &str) -> Result<(), String> {
    match stmt {
        Stmt::Let { ty, value, .. } | Stmt::Const { ty, value, .. } => {
            check_ty_no_generics(ty.as_ref(), path)?;
            check_expr_no_generics(value, path)
        }
        Stmt::Assign { value, .. } => check_expr_no_generics(value, path),
        Stmt::AssignIndex { index, value, .. } => {
            check_expr_no_generics(index, path)?;
            check_expr_no_generics(value, path)
        }
        Stmt::AssignMember { object, value, .. } => {
            check_expr_no_generics(object, path)?;
            check_expr_no_generics(value, path)
        }
        Stmt::Function(f) => check_function_no_generics(f, path),
        Stmt::Return(Some(e)) | Stmt::Throw(e) | Stmt::Expr(e) => check_expr_no_generics(e, path),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => Ok(()),
        Stmt::If { condition, then_branch, else_branch } => {
            check_expr_no_generics(condition, path)?;
            for s in then_branch {
                check_stmt_no_generics(s, path)?;
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    check_stmt_no_generics(s, path)?;
                }
            }
            Ok(())
        }
        Stmt::While { condition, body } => {
            check_expr_no_generics(condition, path)?;
            for s in body {
                check_stmt_no_generics(s, path)?;
            }
            Ok(())
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(i) = init {
                check_stmt_no_generics(i, path)?;
            }
            if let Some(c) = condition {
                check_expr_no_generics(c, path)?;
            }
            if let Some(u) = update {
                check_stmt_no_generics(u, path)?;
            }
            for s in body {
                check_stmt_no_generics(s, path)?;
            }
            Ok(())
        }
        Stmt::ForOf { ty, iterable, body, .. } => {
            check_ty_no_generics(ty.as_ref(), path)?;
            check_expr_no_generics(iterable, path)?;
            for s in body {
                check_stmt_no_generics(s, path)?;
            }
            Ok(())
        }
        Stmt::Switch { discriminant, cases } => {
            check_expr_no_generics(discriminant, path)?;
            for case in cases {
                for v in &case.values {
                    check_expr_no_generics(v, path)?;
                }
                for s in &case.body {
                    check_stmt_no_generics(s, path)?;
                }
            }
            Ok(())
        }
        Stmt::Try { body, catch_body, .. } => {
            for s in body {
                check_stmt_no_generics(s, path)?;
            }
            for s in catch_body {
                check_stmt_no_generics(s, path)?;
            }
            Ok(())
        }
        Stmt::Import { .. } | Stmt::FromImport { .. } | Stmt::ExportSpec(_) | Stmt::Enum { .. } => Ok(()),
        Stmt::ExportDecl(inner) => check_stmt_no_generics(inner, path),
        Stmt::ExportDefault(ExportDefault::Function(f)) => check_function_no_generics(f, path),
        Stmt::ExportDefault(ExportDefault::Expr(e)) => check_expr_no_generics(e, path),
        Stmt::TypeAlias { type_params, ty, .. } => {
            if !type_params.is_empty() {
                // The declaration itself is fine to have around (its own
                // body necessarily mentions its own type params, which
                // aren't a `Type::Generic` usage) — only a concrete,
                // non-generic alias's underlying type needs checking here.
                return Ok(());
            }
            check_ty_no_generics(Some(ty), path)
        }
        Stmt::Interface { type_params, fields, .. } => {
            if !type_params.is_empty() {
                return Ok(());
            }
            for (_, t, _) in fields {
                check_ty_no_generics(Some(t), path)?;
            }
            Ok(())
        }
    }
}

fn check_expr_no_generics(e: &Expr, path: &str) -> Result<(), String> {
    match e {
        Expr::Call { callee, args, type_args } => {
            if matches!(callee.as_ref(), Expr::Ident(n) if n == "json") {
                return Err(json_unsupported_err(path));
            }
            if !type_args.is_empty() {
                let name = match callee.as_ref() {
                    Expr::Ident(n) => n.clone(),
                    _ => "<call>".to_string(),
                };
                return Err(generics_err(path, "call", &name));
            }
            check_expr_no_generics(callee, path)?;
            for a in args {
                check_expr_no_generics(a, path)?;
            }
            Ok(())
        }
        Expr::Unary { operand, .. }
        | Expr::TypeOf(operand)
        | Expr::NonNullAssertion(operand)
        | Expr::AsConst(operand) => check_expr_no_generics(operand, path),
        Expr::Binary { left, right, .. } => {
            check_expr_no_generics(left, path)?;
            check_expr_no_generics(right, path)
        }
        Expr::Member { object, .. } => check_expr_no_generics(object, path),
        Expr::Index { object, index } => {
            check_expr_no_generics(object, path)?;
            check_expr_no_generics(index, path)
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    ArrayElement::Item(e) | ArrayElement::Spread(e) => check_expr_no_generics(e, path)?,
                }
            }
            Ok(())
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    ObjectField::KV(_, e) | ObjectField::Spread(e) => check_expr_no_generics(e, path)?,
                }
            }
            Ok(())
        }
        Expr::AsAssertion { expr, ty } => {
            check_ty_no_generics(Some(ty), path)?;
            check_expr_no_generics(expr, path)
        }
        Expr::Arrow { params, return_type, body } => {
            for p in params {
                check_ty_no_generics(Some(&p.ty), path)?;
            }
            check_ty_no_generics(return_type.as_ref(), path)?;
            match body {
                ArrowBody::Expr(e) => check_expr_no_generics(e, path),
                ArrowBody::Block(stmts) => {
                    for s in stmts {
                        check_stmt_no_generics(s, path)?;
                    }
                    Ok(())
                }
            }
        }
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Ident(_)
        | Expr::Path { .. }
        | Expr::Null
        | Expr::Undefined => Ok(()),
    }
}

/// Write the files for the **Cargo** layout (`bin/<pkg>/Cargo.toml` +
/// `bin/<pkg>/src/<id>.rs`).
fn write_cargo_layout(
    modules: &[Module],
    generated: &[(String, String)],
    bin_dir: &Path,
    user_cargo_toml: &Path,
    root_id: &str,
) -> Result<super::BuildOutput, String> {
    // `<pkg>` is derived from the entry directory's name (not from the
    // root_id) so it stays consistent with `arcis init`'s Cargo.toml.
    let entry_dir = user_cargo_toml.parent().ok_or_else(|| {
        format!(
            "could not determine the directory of `{}`",
            user_cargo_toml.display()
        )
    })?;
    let pkg_name = entry_dir
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != ".")
        .unwrap_or(root_id)
        .to_string();
    let target_dir = bin_dir.join(&pkg_name);

    // If a stale file from a previous rustc-mode build sits here, clean it.
    if target_dir.exists() && !target_dir.is_dir() {
        fs::remove_file(&target_dir).map_err(|e| {
            format!("could not clean `{}`: {}", target_dir.display(), e)
        })?;
    }
    let src_dir = target_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| format!("could not create `{}`: {}", src_dir.display(), e))?;

    // Copy the user's Cargo.toml (otherwise generate a minimal one).
    let dest_cargo = target_dir.join("Cargo.toml");
    if user_cargo_toml != dest_cargo {
        fs::copy(user_cargo_toml, &dest_cargo).map_err(|e| {
            format!(
                "could not copy `{}` to `{}`: {}",
                user_cargo_toml.display(),
                dest_cargo.display(),
                e
            )
        })?;
    }

    // Force the Cargo sub-project to be its own workspace. Cargo
    // auto-detects a workspace by walking up from the Cargo.toml, and
    // by default searches for `src/*.rs` at the workspace root — not
    // where the Cargo.toml lives. Adding an empty `[workspace]` section
    // makes the sub-project self-contained.
    let mut cargo_contents = fs::read_to_string(&dest_cargo)
        .map_err(|e| format!("could not read `{}`: {}", dest_cargo.display(), e))?;
    // `json(...)` needs `serde`/`serde_json` at runtime — inject them
    // automatically (rather than requiring the user to hand-uncomment the
    // lines `arcis init` scaffolds) so `json(...)` feels like a real
    // builtin, not something that needs manual dependency wiring.
    if modules.iter().any(|m| arcis_ast::contains_json_call(&m.program)) {
        cargo_contents = ensure_json_deps(&cargo_contents);
    }
    if !cargo_contents.ends_with('\n') {
        cargo_contents.push('\n');
    }
    cargo_contents.push_str("\n[workspace]\n");
    fs::write(&dest_cargo, cargo_contents)
        .map_err(|e| format!("could not write `{}`: {}", dest_cargo.display(), e))?;

    for (id, src) in generated {
        let p = src_dir.join(format!("{}.rs", id));
        fs::write(&p, src)
            .map_err(|e| format!("could not write `{}`: {}", p.display(), e))?;
    }

    Ok(super::BuildOutput {
        sources: generated.to_vec(),
        root_id: root_id.to_string(),
        cargo_dir: Some(target_dir),
        pkg_name: Some(pkg_name),
        backend: super::Backend::Rust,
    })
}

/// Write the files for the **direct rustc** layout (`bin/<id>.rs`).
fn write_rustc_layout(
    modules: &[Module],
    generated: &[(String, String)],
    bin_dir: &Path,
    root_id: &str,
) -> Result<super::BuildOutput, String> {
    // Without a Cargo.toml: if the program imports external crates, give a
    // clear message instead of letting rustc fail cryptically.
    if has_crate_import(modules) {
        return Err(
            "to use Rust crates (`from \"crate:<name>\"`) you need a \
             `Cargo.toml` next to `main.tsr`. \
             Create one (or run `arcis init` again) and uncomment the dependency."
                .to_string(),
        );
    }
    // Same story for `json(...)` — its runtime deserialization needs
    // `serde`/`serde_json`, which only a Cargo-layout build can carry (bare
    // `rustc` can't fetch crates from crates.io at all).
    if has_json_builtin_call(modules) {
        return Err(
            "to use json(...) you need a `Cargo.toml` next to `main.tsr` \
             (it needs `serde`/`serde_json` at runtime, auto-added once a \
             Cargo.toml exists). Create one (or run `arcis init` again)."
                .to_string(),
        );
    }
    for (id, src) in generated {
        let p = bin_dir.join(format!("{}.rs", id));
        fs::write(&p, src)
            .map_err(|e| format!("could not write `{}`: {}", p.display(), e))?;
    }
    Ok(super::BuildOutput {
        sources: generated.to_vec(),
        root_id: root_id.to_string(),
        cargo_dir: None,
        pkg_name: None,
        backend: super::Backend::Rust,
    })
}

/// Returns `true` if any module imports a specifier with the `crate:` prefix.
fn has_crate_import(modules: &[Module]) -> bool {
    modules.iter().any(|m| {
        m.program.stmts.iter().any(|s| match s {
            Stmt::Import { module, .. } | Stmt::FromImport { module, .. } => {
                module.first().map_or(false, |seg| seg.starts_with("crate:"))
            }
            _ => false,
        })
    })
}

/// `true` if any module calls the `json(...)` builtin anywhere.
fn has_json_builtin_call(modules: &[Module]) -> bool {
    modules.iter().any(|m| arcis_ast::contains_json_call(&m.program))
}

/// Ensure `serde`/`serde_json` are present as active (non-comment)
/// dependencies in a Cargo.toml's text, inserting the exact known-good
/// lines `arcis init` itself scaffolds (commented out there) if either is
/// missing. A narrow, line-based text transform — not a general TOML
/// editor — deliberately, since the only two lines ever inserted are fixed,
/// already-tested strings; see this module's design notes for why a
/// TOML-parsing dependency wasn't added for this.
fn ensure_json_deps(contents: &str) -> String {
    let has_active_dep = |dep: &str| {
        contents.lines().any(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && (t.starts_with(&format!("{dep} ")) || t.starts_with(&format!("{dep}=")))
        })
    };
    let mut needed = String::new();
    if !has_active_dep("serde") {
        needed.push_str("serde = { version = \"1\", features = [\"derive\"] }\n");
    }
    if !has_active_dep("serde_json") {
        needed.push_str("serde_json = \"1\"\n");
    }
    if needed.is_empty() {
        return contents.to_string();
    }
    // Find an ACTIVE `[dependencies]` table header — a commented-out one
    // (`# [dependencies]`, as `arcis init`'s own template scaffolds) must
    // not be treated as a real table, or the injected lines would land
    // with no active header above them.
    let active_header = contents.lines().find_map(|l| {
        if l.trim() == "[dependencies]" {
            Some(l)
        } else {
            None
        }
    });
    match active_header.and_then(|header_line| contents.find(header_line)) {
        Some(pos) => {
            let insert_at = contents[pos..].find('\n').map_or(contents.len(), |i| pos + i + 1);
            let mut out = String::with_capacity(contents.len() + needed.len());
            out.push_str(&contents[..insert_at]);
            out.push_str(&needed);
            out.push_str(&contents[insert_at..]);
            out
        }
        None => {
            let mut out = contents.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("\n[dependencies]\n");
            out.push_str(&needed);
            out
        }
    }
}

#[cfg(test)]
mod json_deps_tests {
    use super::ensure_json_deps;

    #[test]
    fn adds_both_deps_to_a_fresh_arcis_init_template() {
        let input = "[package]\nname = \"x\"\n\n# [dependencies]\n# serde = { version = \"1\", features = [\"derive\"] }\n# serde_json = \"1\"\n";
        let out = ensure_json_deps(input);
        assert!(out.contains("\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n"));
    }

    #[test]
    fn inserts_after_an_existing_dependencies_table_without_disturbing_other_entries() {
        let input = "[package]\nname = \"x\"\n\n[dependencies]\ntempfile = \"3\"\n";
        let out = ensure_json_deps(input);
        assert!(out.contains("[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\ntempfile = \"3\"\n"));
    }

    #[test]
    fn leaves_contents_unchanged_when_both_deps_already_active() {
        let input = "[package]\nname = \"x\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n";
        assert_eq!(ensure_json_deps(input), input);
    }

    #[test]
    fn appends_a_fresh_dependencies_table_when_none_exists() {
        let input = "[package]\nname = \"x\"\n";
        let out = ensure_json_deps(input);
        assert!(out.ends_with("\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n"));
    }

    #[test]
    fn only_adds_the_missing_one_when_one_dep_is_already_active() {
        let input = "[package]\nname = \"x\"\n\n[dependencies]\nserde_json = \"1\"\n";
        let out = ensure_json_deps(input);
        assert!(out.contains("[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n"));
    }
}

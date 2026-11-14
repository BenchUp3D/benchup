//! Replays the reference solutions of the agent evaluation cases
//! (`evals/cases.json`) with the real binary, so a change to the tools that
//! breaks what agents did successfully fails here, without running agents.
//! See `evals/README.md`.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn benchup(args: &[&str]) -> (Value, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_benchup")).args(args).output().expect("run benchup");
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!("benchup {args:?}: stdout is not JSON: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
    });
    (value, output.status.success())
}

fn check(case: &str, file: &str, expect: &Value) {
    let (report, _) = benchup(&["call", file, "check_model"]);
    let kinds: Vec<&str> = report["issues"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap()).collect();
    let context = || format!("{case}: {}", serde_json::to_string_pretty(&report["issues"]).unwrap());
    if let Some(passed) = expect.get("passed") {
        assert_eq!(&report["passed"], passed, "{}", context());
    }
    if let Some(errors) = expect.get("errors") {
        assert_eq!(&report["errors"], errors, "{}", context());
    }
    if let Some(allowed) = expect.get("allowed_kinds").and_then(Value::as_array) {
        let allowed: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
        assert!(kinds.iter().all(|k| allowed.contains(k)), "{}", context());
    }
    if let Some(required) = expect.get("required_kinds").and_then(Value::as_array) {
        assert!(required.iter().filter_map(Value::as_str).all(|k| kinds.contains(&k)), "{}", context());
    }
    assert_eq!(report["joints"]["failing"], json!([]), "{}", context());
    let (scene, _) = benchup(&["scene", file, r#"{"sections":["dimensions","texts"]}"#]);
    for (key, section) in [("min_dimensions", "dimensions"), ("min_texts", "texts")] {
        if let Some(min) = expect.get(key).and_then(Value::as_u64) {
            assert!(scene[section].as_array().unwrap().len() as u64 >= min, "{case}: fewer {section} than {min}");
        }
    }
}

#[test]
fn reference_solutions_of_the_agent_cases_still_apply() {
    let root = root();
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(root.join("crates/benchup-cli/evals/cases.json")).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut replayed = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let Some(reference) = case.get("reference") else { continue };
        let id = case["id"].as_str().unwrap();
        let file = dir.path().join(format!("{id}-{}", case["file"].as_str().unwrap()));
        let file = file.to_str().unwrap();
        match manifest["seeds"][case["seed"].as_str().unwrap()].as_str() {
            Some(seed) => {
                std::fs::copy(root.join(seed), file).unwrap();
            }
            None => assert!(benchup(&["new", file, "--name", id]).1, "{id}: benchup new failed"),
        }
        for batch in reference["batches"].as_array().unwrap() {
            let path = root.join("crates/benchup-cli/evals/reference").join(id).join(batch.as_str().unwrap());
            let (result, ok) = benchup(&["call", file, "--batch", path.to_str().unwrap(), "--brief"]);
            assert!(ok, "{id}/{batch}: {}", serde_json::to_string_pretty(&result).unwrap());
        }
        check(id, file, &reference["expect"]);
        replayed += 1;
    }
    assert!(replayed >= 15, "only {replayed} cases have a reference solution");
}

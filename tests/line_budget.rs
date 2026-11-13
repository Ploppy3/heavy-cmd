//! The line budget: 450 lines per Rust file in src/ and tests/, so a file stays one thing.

use std::path::Path;

const BUDGET: usize = 450;

fn over(dir: &Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            over(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let lines = std::fs::read_to_string(&path).unwrap().lines().count();
            if lines > BUDGET {
                found.push(format!("{}: {lines} lines", path.display()));
            }
        }
    }
}

#[test]
fn every_rust_file_keeps_to_450_lines() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for dir in ["src", "tests"] {
        over(&root.join(dir), &mut found);
    }
    assert!(found.is_empty(), "over {BUDGET} lines: {found:?}");
}

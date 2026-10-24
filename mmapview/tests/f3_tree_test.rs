use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tmpfile() -> String {
    let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("/tmp/mmap_f3_test_{}.mms", n)
}

#[test]
fn test_tree_viewer_output() {
    let path = tmpfile();
    let _ = fs::remove_file(&path);

    // Build a small binary and run it to create test data with embedded schema
    let status = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "mmapview",
            "--example",
            "f3_test_data",
            "--",
            &path,
        ])
        .status()
        .expect("failed to run f3_test_data example");
    if !status.success() {
        panic!("f3_test_data example failed");
    }

    // Now run mmapview --tree on it (simulating MC F3 view)
    let output = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "mmapview",
            "--",
            "--tree",
            "--depth=5",
            "--max-array=10",
            &path,
        ])
        .output()
        .expect("failed to run mmapview --tree");
    assert!(
        output.status.success(),
        "mmapview --tree failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let out = String::from_utf8_lossy(&output.stdout);
    println!("=== TREE OUTPUT ===\n{}", out);

    // Verify key elements appear in the tree
    assert!(out.contains("ItemCatalog"), "missing root name");
    assert!(out.contains("version"), "missing version field");
    assert!(out.contains("items"), "missing items field");
    assert!(out.contains("[3 elements]"), "missing array count");
    assert!(out.contains("Sword"), "missing item name");
    assert!(out.contains("price"), "missing price field");
    assert!(out.contains("tags"), "missing tags field");
    assert!(out.contains("metadata"), "missing metadata field");
    assert!(out.contains("generated_at"), "missing generated_at field");

    fs::remove_file(&path).ok();
}

#[test]
fn test_tree_viewer_auto_detect() {
    // Simulate MC pipe (stdout not a terminal)
    let path = tmpfile();

    let status = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "mmapview",
            "--example",
            "f3_test_data",
            "--",
            &path,
        ])
        .status()
        .expect("failed to run f3_test_data example");
    if !status.success() {
        panic!("f3_test_data example failed");
    }

    // Pipe stdout → auto-detect tree mode
    let output = Command::new("cargo")
        .args(["run", "--quiet", "-p", "mmapview", "--", "--depth=2", &path])
        .output()
        .expect("failed to run mmapview (auto)");
    assert!(
        output.status.success(),
        "mmapview auto-tree failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let out = String::from_utf8_lossy(&output.stdout);
    println!("=== AUTO TREE OUTPUT ===\n{}", out);

    assert!(out.contains("ItemCatalog"), "auto: missing root");
    assert!(out.contains("version"), "auto: missing version");
    assert!(out.contains("items"), "auto: missing items");

    fs::remove_file(&path).ok();
}

#[test]
fn test_tree_viewer_enum_and_option() {
    let path = tmpfile();

    let status = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "mmapview",
            "--example",
            "f3_test_data",
            "--",
            &path,
        ])
        .status()
        .expect("failed to run f3_test_data example");
    if !status.success() {
        panic!("f3_test_data example failed");
    }

    let output = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "mmapview",
            "--",
            "--tree",
            "--depth=5",
            "--max-array=10",
            &path,
        ])
        .output()
        .expect("failed to run mmapview --tree");
    assert!(
        output.status.success(),
        "mmapview --tree failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let out = String::from_utf8_lossy(&output.stdout);
    println!("=== ENUM/OPTION TREE OUTPUT ===\n{}", out);

    // Check enum variant data
    assert!(out.contains("category"), "missing category field");

    fs::remove_file(&path).ok();
}

use std::path::PathBuf;
use std::process::Command;

fn temp_db(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sauron-clip-cli-{name}-{}.sqlite3",
        std::process::id()
    ))
}

#[test]
fn standalone_cli_preserves_values_json_and_exit_codes() {
    let db = temp_db("standalone");
    let _ = std::fs::remove_file(&db);
    let clip = env!("CARGO_BIN_EXE_clip");

    let put = Command::new(clip)
        .args(["--db", db.to_str().unwrap(), "put", "project.state"])
        .arg("line one\nline two\n")
        .args(["--namespace", "project", "--validated", "--json"])
        .output()
        .unwrap();
    assert!(
        put.status.success(),
        "{}",
        String::from_utf8_lossy(&put.stderr)
    );
    let record: serde_json::Value = serde_json::from_slice(&put.stdout).unwrap();
    assert_eq!(record["version"], 1);
    assert_eq!(record["value"], "line one\nline two\n");

    let copy = Command::new(clip)
        .args(["--db", db.to_str().unwrap(), "copy", "project.state"])
        .output()
        .unwrap();
    assert!(copy.status.success());
    assert_eq!(copy.stdout, b"line one\nline two\n");

    let missing = Command::new(clip)
        .args(["--db", db.to_str().unwrap(), "get", "missing", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(error["error"], "not_found");

    let rejected = Command::new(clip)
        .args([
            "--db",
            db.to_str().unwrap(),
            "put",
            "unvalidated",
            "value",
            "--pin",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(error["ok"], false);
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("pinning requires"));

    let _ = std::fs::remove_file(db);
}

#[test]
fn sauron_clip_routes_to_the_same_store() {
    let db = temp_db("sauron");
    let _ = std::fs::remove_file(&db);
    let sauron = env!("CARGO_BIN_EXE_sauron");
    let clip = env!("CARGO_BIN_EXE_clip");

    let put = Command::new(sauron)
        .args([
            "clip",
            "--db",
            db.to_str().unwrap(),
            "put",
            "shared.key",
            "shared value",
        ])
        .output()
        .unwrap();
    assert!(
        put.status.success(),
        "{}",
        String::from_utf8_lossy(&put.stderr)
    );

    let get = Command::new(clip)
        .args(["--db", db.to_str().unwrap(), "get", "shared.key"])
        .output()
        .unwrap();
    assert!(get.status.success());
    assert_eq!(get.stdout, b"shared value\n");

    let _ = std::fs::remove_file(db);
}

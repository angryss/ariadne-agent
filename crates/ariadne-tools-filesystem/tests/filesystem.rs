use ariadne_core::Tool;
use ariadne_tools_filesystem::{FileSystemConfig, FileSystemToolset};
use serde_json::json;

#[cfg(unix)]
fn execute_with_timeout(
    workspace: &std::path::Path,
    name: &'static str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let workspace = workspace.to_owned();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(async move {
            let tools = FileSystemToolset::new(FileSystemConfig::new(workspace))
                .unwrap()
                .tools();
            tool(&tools, name)
                .execute(arguments)
                .await
                .map_err(|error| error.to_string())
        });
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap_or_else(|_| panic!("{name} blocked on a special file"))
}

#[cfg(unix)]
fn create_fifo(path: &std::path::Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn tool<'a>(tools: &'a [std::sync::Arc<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|tool| tool.definition().name == name)
        .map(std::convert::AsRef::as_ref)
        .unwrap()
}

#[tokio::test]
async fn read_file_is_scoped_to_the_workspace_and_denies_secrets() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("README.md"), "# Ariadne\n").unwrap();
    std::fs::write(workspace.path().join(".env"), "TOKEN=secret\n").unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), "outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), workspace.path().join("escape")).unwrap();

    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();
    let read = tool(&tools, "read_file");

    let result = read.execute(json!({"path": "README.md"})).await.unwrap();
    assert_eq!(result["content"], "# Ariadne\n");
    assert!(result["sha256"].as_str().unwrap().len() == 64);

    let traversal = read
        .execute(json!({"path": "../outside"}))
        .await
        .unwrap_err();
    assert!(traversal.to_string().contains("outside the workspace"));
    let secret = read.execute(json!({"path": ".env"})).await.unwrap_err();
    assert!(secret.to_string().contains("denied by filesystem policy"));

    #[cfg(unix)]
    {
        let symlink = read.execute(json!({"path": "escape"})).await.unwrap_err();
        assert!(symlink.to_string().contains("symlink"));
    }
}

#[cfg(unix)]
#[test]
fn read_file_rejects_a_fifo_without_blocking() {
    let workspace = tempfile::tempdir().unwrap();
    create_fifo(&workspace.path().join("pipe"));

    let error =
        execute_with_timeout(workspace.path(), "read_file", json!({"path": "pipe"})).unwrap_err();

    assert!(error.contains("not a regular file"), "{error}");
}

#[cfg(unix)]
#[test]
fn write_and_edit_reject_a_fifo_without_blocking() {
    let workspace = tempfile::tempdir().unwrap();
    create_fifo(&workspace.path().join("pipe"));

    for (name, arguments) in [
        (
            "write_file",
            json!({"path": "pipe", "content": "replacement"}),
        ),
        (
            "edit_file",
            json!({"path": "pipe", "old_text": "old", "new_text": "new"}),
        ),
    ] {
        let error = execute_with_timeout(workspace.path(), name, arguments).unwrap_err();
        assert!(error.contains("not a regular file"), "{name}: {error}");
    }
}

#[cfg(unix)]
#[test]
fn file_info_rejects_a_fifo_without_blocking() {
    let workspace = tempfile::tempdir().unwrap();
    create_fifo(&workspace.path().join("pipe"));

    let error =
        execute_with_timeout(workspace.path(), "file_info", json!({"path": "pipe"})).unwrap_err();

    assert!(error.contains("not a regular file"), "{error}");
}

#[cfg(unix)]
#[test]
fn list_directory_skips_a_fifo_without_blocking() {
    let workspace = tempfile::tempdir().unwrap();
    create_fifo(&workspace.path().join("pipe"));

    let listed =
        execute_with_timeout(workspace.path(), "list_directory", json!({"path": "."})).unwrap();

    assert!(listed["entries"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn traversal_skips_a_fifo_without_blocking() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("regular.txt"), "needle\n").unwrap();
    create_fifo(&workspace.path().join("pipe"));

    let found = execute_with_timeout(
        workspace.path(),
        "find_files",
        json!({"path": ".", "pattern": "*"}),
    )
    .unwrap();
    let searched = execute_with_timeout(
        workspace.path(),
        "search_files",
        json!({"path": ".", "pattern": "needle"}),
    )
    .unwrap();

    assert_eq!(found["paths"], json!(["regular.txt"]));
    assert_eq!(searched["matches"][0]["path"], "regular.txt");
}

#[tokio::test]
async fn write_and_edit_enforce_hash_read_only_and_protected_policies() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join(".git")).unwrap();
    std::fs::write(workspace.path().join(".git/config"), "protected").unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();

    let written = tool(&tools, "write_file")
        .execute(json!({"path": "notes.txt", "content": "one\n"}))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("notes.txt")).unwrap(),
        "one\n"
    );

    let stale = tool(&tools, "edit_file")
        .execute(json!({
            "path": "notes.txt",
            "old_text": "one",
            "new_text": "two",
            "expected_sha256": "stale"
        }))
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("changed since it was read"));

    tool(&tools, "edit_file")
        .execute(json!({
            "path": "notes.txt",
            "old_text": "one",
            "new_text": "two",
            "expected_sha256": written["sha256"]
        }))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("notes.txt")).unwrap(),
        "two\n"
    );

    let protected = tool(&tools, "write_file")
        .execute(json!({"path": ".git/config", "content": "clobbered"}))
        .await
        .unwrap_err();
    assert!(
        protected
            .to_string()
            .contains("protected by filesystem policy")
    );

    let mut read_only = FileSystemConfig::new(workspace.path());
    read_only.read_only = true;
    let read_only_tools = FileSystemToolset::new(read_only).unwrap().tools();
    let denied = tool(&read_only_tools, "write_file")
        .execute(json!({"path": "other.txt", "content": "no"}))
        .await
        .unwrap_err();
    assert!(denied.to_string().contains("read-only"));
}

#[tokio::test]
async fn conditional_write_to_missing_file_is_side_effect_free() {
    let workspace = tempfile::tempdir().unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();
    let path = workspace.path().join("missing.txt");

    let error = tool(&tools, "write_file")
        .execute(json!({
            "path": "missing.txt",
            "content": "replacement",
            "expected_sha256": "stale"
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("changed since it was read"));
    assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn write_rechecks_policy_after_resolving_intermediate_symlinks() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join(".git")).unwrap();
    std::fs::write(workspace.path().join(".git/config"), "protected").unwrap();
    std::os::unix::fs::symlink(".git", workspace.path().join("alias")).unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();

    let error = tool(&tools, "write_file")
        .execute(json!({"path": "alias/config", "content": "clobbered"}))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("symlink"));
    let create_error = tool(&tools, "write_file")
        .execute(json!({"path": "alias/new-config", "content": "created"}))
        .await
        .unwrap_err();

    assert!(create_error.to_string().contains("symlink"));
    assert!(!workspace.path().join(".git/new-config").exists());
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".git/config")).unwrap(),
        "protected"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn every_tool_rejects_symlinks_in_any_path_component() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("safe")).unwrap();
    std::fs::write(workspace.path().join("safe/file.txt"), "safe").unwrap();
    std::os::unix::fs::symlink("safe", workspace.path().join("alias")).unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();

    for (name, arguments) in [
        ("read_file", json!({"path": "alias/file.txt"})),
        (
            "write_file",
            json!({"path": "alias/new.txt", "content": "x"}),
        ),
        (
            "edit_file",
            json!({"path": "alias/file.txt", "old_text": "safe", "new_text": "x"}),
        ),
        ("list_directory", json!({"path": "alias"})),
        ("find_files", json!({"path": "alias", "pattern": "*"})),
        ("search_files", json!({"path": "alias", "pattern": "safe"})),
        ("create_directory", json!({"path": "alias/new"})),
        ("file_info", json!({"path": "alias/file.txt"})),
    ] {
        let error = tool(&tools, name).execute(arguments).await.unwrap_err();
        assert!(error.to_string().contains("symlink"), "{name}: {error}");
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writes_cannot_be_raced_through_an_in_root_git_symlink() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("safe")).unwrap();
    std::fs::create_dir(workspace.path().join(".git")).unwrap();
    std::fs::write(workspace.path().join("safe/config"), "safe").unwrap();
    std::fs::write(workspace.path().join(".git/config"), "protected").unwrap();
    let slot = workspace.path().join("slot");
    let parked = workspace.path().join("parked");
    std::fs::rename(workspace.path().join("safe"), &slot).unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();
    let stop = Arc::new(AtomicBool::new(false));
    let toggler_stop = Arc::clone(&stop);
    let toggler = std::thread::spawn(move || {
        while !toggler_stop.load(Ordering::Relaxed) {
            if std::fs::rename(&slot, &parked).is_ok() {
                let _ = std::os::unix::fs::symlink(".git", &slot);
                std::thread::yield_now();
                let _ = std::fs::remove_file(&slot);
                let _ = std::fs::rename(&parked, &slot);
            }
        }
    });

    for _ in 0..20_000 {
        let _ = tool(&tools, "write_file")
            .execute(json!({"path": "slot/config", "content": "safe"}))
            .await;
        if std::fs::read_to_string(workspace.path().join(".git/config")).unwrap() != "protected" {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    toggler.join().unwrap();

    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".git/config")).unwrap(),
        "protected"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_cannot_be_raced_through_a_replaced_parent_directory() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let slot = workspace.path().join("slot");
    let parked = workspace.path().join("parked");
    std::fs::create_dir(&slot).unwrap();
    std::fs::write(slot.join("target"), "inside").unwrap();
    std::fs::write(outside.path().join("target"), "outside").unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();
    let stop = Arc::new(AtomicBool::new(false));
    let toggler_stop = Arc::clone(&stop);
    let outside_path = outside.path().to_owned();
    let toggler = std::thread::spawn(move || {
        while !toggler_stop.load(Ordering::Relaxed) {
            if std::fs::rename(&slot, &parked).is_ok() {
                let _ = std::os::unix::fs::symlink(&outside_path, &slot);
                std::thread::yield_now();
                let _ = std::fs::remove_file(&slot);
                let _ = std::fs::rename(&parked, &slot);
            }
        }
    });

    for _ in 0..20_000 {
        let _ = tool(&tools, "write_file")
            .execute(json!({"path": "slot/target", "content": "inside"}))
            .await;
        if std::fs::read_to_string(outside.path().join("target")).unwrap() != "outside" {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    toggler.join().unwrap();

    assert_eq!(
        std::fs::read_to_string(outside.path().join("target")).unwrap(),
        "outside"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_file_cannot_be_raced_through_a_replaced_parent_directory() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let slot = workspace.path().join("slot");
    let parked = workspace.path().join("parked");
    std::fs::create_dir(&slot).unwrap();
    std::fs::write(slot.join("target"), "inside").unwrap();
    std::fs::write(outside.path().join("target"), "outside").unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();
    let stop = Arc::new(AtomicBool::new(false));
    let toggler_stop = Arc::clone(&stop);
    let outside_path = outside.path().to_owned();
    let toggler = std::thread::spawn(move || {
        while !toggler_stop.load(Ordering::Relaxed) {
            if std::fs::rename(&slot, &parked).is_ok() {
                let _ = std::os::unix::fs::symlink(&outside_path, &slot);
                std::thread::yield_now();
                let _ = std::fs::remove_file(&slot);
                let _ = std::fs::rename(&parked, &slot);
            }
        }
    });

    let mut escaped = false;
    for _ in 0..20_000 {
        if let Ok(result) = tool(&tools, "read_file")
            .execute(json!({"path": "slot/target"}))
            .await
            && result["content"] == "outside"
        {
            escaped = true;
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    toggler.join().unwrap();

    assert!(!escaped, "read escaped the workspace capability root");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_directory_cannot_be_raced_through_a_replaced_parent_directory() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let slot = workspace.path().join("slot");
    let parked = workspace.path().join("parked");
    std::fs::create_dir(&slot).unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();
    let stop = Arc::new(AtomicBool::new(false));
    let toggler_stop = Arc::clone(&stop);
    let outside_path = outside.path().to_owned();
    let toggler = std::thread::spawn(move || {
        while !toggler_stop.load(Ordering::Relaxed) {
            if std::fs::rename(&slot, &parked).is_ok() {
                let _ = std::os::unix::fs::symlink(&outside_path, &slot);
                std::thread::yield_now();
                let _ = std::fs::remove_file(&slot);
                let _ = std::fs::rename(&parked, &slot);
            }
        }
    });

    for index in 0..20_000 {
        let _ = tool(&tools, "create_directory")
            .execute(json!({"path": format!("slot/new-{index}")}))
            .await;
        if outside.path().read_dir().unwrap().next().is_some() {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    toggler.join().unwrap();

    assert_eq!(outside.path().read_dir().unwrap().count(), 0);
}

#[tokio::test]
async fn nested_allowlist_authorizes_matching_files_not_their_ancestors_or_siblings() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("src/nested")).unwrap();
    std::fs::create_dir_all(workspace.path().join("src/secrets-store")).unwrap();
    std::fs::write(
        workspace.path().join("src/nested/main.rs"),
        "fn allowed() {}\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("src/nested/notes.txt"),
        "not allowed\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("src/secrets-store/hidden.rs"),
        "fn secret() {}\n",
    )
    .unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.allowed_patterns = vec!["src/**/*.rs".to_owned()];
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let read = tool(&tools, "read_file")
        .execute(json!({"path": "src/nested/main.rs"}))
        .await
        .unwrap();
    assert_eq!(read["content"], "fn allowed() {}\n");

    for path in ["src/nested/notes.txt", "src/secrets-store/hidden.rs"] {
        let error = tool(&tools, "read_file")
            .execute(json!({"path": path}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("denied by filesystem policy"));
    }

    let found = tool(&tools, "find_files")
        .execute(json!({"path": "src", "pattern": "*.rs"}))
        .await
        .unwrap();
    assert_eq!(found["paths"], json!(["src/nested/main.rs"]));

    let searched = tool(&tools, "search_files")
        .execute(json!({"path": "src", "pattern": "allowed"}))
        .await
        .unwrap();
    assert_eq!(searched["matches"][0]["path"], "src/nested/main.rs");

    let listed = tool(&tools, "list_directory")
        .execute(json!({"path": "src/nested"}))
        .await
        .unwrap();
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(listed["entries"][0]["path"], "src/nested/main.rs");

    tool(&tools, "write_file")
        .execute(json!({"path": "src/nested/new.rs", "content": "fn new() {}\n"}))
        .await
        .unwrap();
    let error = tool(&tools, "write_file")
        .execute(json!({"path": "src/nested/new.txt", "content": "denied\n"}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("denied by filesystem policy"));

    let error = tool(&tools, "create_directory")
        .execute(json!({"path": "src/generated"}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("denied by filesystem policy"));
}

#[tokio::test]
async fn file_info_applies_final_target_policy_to_the_workspace_root() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.allowed_patterns = vec!["src/**/*.rs".to_owned()];
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let error = tool(&tools, "file_info")
        .execute(json!({"path": "."}))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("denied by filesystem policy"));
}

#[tokio::test]
async fn list_find_and_search_return_bounded_policy_filtered_results() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("src")).unwrap();
    std::fs::write(
        workspace.path().join("src/main.rs"),
        "fn main() { println!(\"Ariadne\"); }\n",
    )
    .unwrap();
    std::fs::write(workspace.path().join("README.md"), "# Ariadne\n").unwrap();
    std::fs::write(workspace.path().join(".env"), "ARIADNE_API_KEY=secret\n").unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();

    let listed = tool(&tools, "list_directory")
        .execute(json!({"path": "."}))
        .await
        .unwrap();
    assert!(
        listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["path"] != ".env")
    );

    let found = tool(&tools, "find_files")
        .execute(json!({"path": ".", "pattern": "*.rs"}))
        .await
        .unwrap();
    assert_eq!(found["paths"], json!(["src/main.rs"]));

    let searched = tool(&tools, "search_files")
        .execute(json!({"path": ".", "pattern": "Ariadne", "include_glob": "*.rs"}))
        .await
        .unwrap();
    assert_eq!(searched["matches"][0]["path"], "src/main.rs");
    assert_eq!(searched["matches"][0]["line"], 1);
}

#[tokio::test]
async fn traversal_budget_is_spent_as_entries_are_processed() {
    let workspace = tempfile::tempdir().unwrap();
    for index in 0..128 {
        std::fs::write(
            workspace.path().join(format!("entry-{index:03}")),
            "unrelated",
        )
        .unwrap();
    }
    let first = std::fs::read_dir(workspace.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let later = (0..128)
        .map(|index| format!("entry-{index:03}"))
        .find(|name| name > &first)
        .expect("test fixture needs an entry lexically after the first native entry");
    for entry in std::fs::read_dir(workspace.path()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if name != first && name != later {
            std::fs::remove_file(path).unwrap();
        }
    }
    std::fs::remove_file(workspace.path().join(&first)).unwrap();
    std::fs::create_dir(workspace.path().join(&first)).unwrap();
    std::fs::write(workspace.path().join(&first).join("hit.rs"), "hit").unwrap();

    let mut config = FileSystemConfig::new(workspace.path());
    config.max_results = 1;
    config.max_traversal_files = 2;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let found = tool(&tools, "find_files")
        .execute(json!({"path": ".", "pattern": "*.rs"}))
        .await
        .unwrap();

    assert_eq!(found["paths"], json!([format!("{first}/hit.rs")]));
}

#[tokio::test]
async fn find_files_stops_after_the_configured_traversal_file_budget() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("00.txt"), "first").unwrap();
    std::fs::write(workspace.path().join("01.rs"), "second").unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.max_traversal_files = 1;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let found = tool(&tools, "find_files")
        .execute(json!({"path": ".", "pattern": "*.rs"}))
        .await
        .unwrap();

    assert_eq!(found["paths"], json!([]));
}

#[tokio::test]
async fn find_files_stops_at_the_configured_traversal_depth() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("nested")).unwrap();
    std::fs::write(workspace.path().join("root.rs"), "root").unwrap();
    std::fs::write(workspace.path().join("nested/deep.rs"), "deep").unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.max_traversal_depth = 1;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let found = tool(&tools, "find_files")
        .execute(json!({"path": ".", "pattern": "*.rs"}))
        .await
        .unwrap();

    assert_eq!(found["paths"], json!(["root.rs"]));
}

#[tokio::test]
async fn search_files_stops_at_the_configured_aggregate_byte_budget() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("00.txt"), "hit\n").unwrap();
    std::fs::write(workspace.path().join("01.txt"), "hit\n").unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.max_search_bytes = 4;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let searched = tool(&tools, "search_files")
        .execute(json!({"path": ".", "pattern": "hit"}))
        .await
        .unwrap();

    assert_eq!(searched["matches"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn traversal_budget_counts_directories_before_descending() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("00-dir")).unwrap();
    std::fs::write(workspace.path().join("00-dir/hidden.rs"), "hidden").unwrap();
    std::fs::write(workspace.path().join("01.rs"), "visible").unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.max_traversal_files = 1;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let found = tool(&tools, "find_files")
        .execute(json!({"path": ".", "pattern": "*.rs"}))
        .await
        .unwrap();

    assert_eq!(found["paths"], json!([]));
}

#[tokio::test]
async fn search_reads_no_more_than_the_remaining_aggregate_byte_budget() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("large.txt"),
        "hit followed by excess bytes",
    )
    .unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.max_search_bytes = 3;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let searched = tool(&tools, "search_files")
        .execute(json!({"path": ".", "pattern": "hit"}))
        .await
        .unwrap();

    assert_eq!(searched["matches"][0]["text"], "hit");
}

#[tokio::test]
async fn write_hash_verification_respects_the_read_byte_limit() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("large.txt"), "12345").unwrap();
    let mut config = FileSystemConfig::new(workspace.path());
    config.max_read_bytes = 4;
    let tools = FileSystemToolset::new(config).unwrap().tools();

    let error = tool(&tools, "write_file")
        .execute(json!({
            "path": "large.txt",
            "content": "replacement",
            "expected_sha256": "stale"
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("exceeds the 4-byte read limit"));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("large.txt")).unwrap(),
        "12345"
    );
}

#[tokio::test]
async fn create_directory_and_file_info_stay_inside_the_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let tools = FileSystemToolset::new(FileSystemConfig::new(workspace.path()))
        .unwrap()
        .tools();

    tool(&tools, "create_directory")
        .execute(json!({"path": "nested/output"}))
        .await
        .unwrap();
    let info = tool(&tools, "file_info")
        .execute(json!({"path": "nested/output"}))
        .await
        .unwrap();

    assert_eq!(info["kind"], "directory");
    assert!(workspace.path().join("nested/output").is_dir());
}

use rynna_config::memory::{
    HINDSIGHT_CLOUD_URL, HindsightDeployment, MemorySettings, MemorySettingsStore,
};

fn hindsight(base: &str, cloud: bool, key: Option<&str>) -> MemorySettings {
    MemorySettings::Hindsight {
        deployment: if cloud {
            HindsightDeployment::Cloud
        } else {
            HindsightDeployment::SelfHosted
        },
        api_base: base.into(),
        bank_id: "rynna".into(),
        api_key: key.map(str::to_owned),
    }
}

#[test]
fn default_none_and_cloud_key_round_trip_preserve_and_disable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("memory.toml");
    let store = MemorySettingsStore::new(&path);
    assert!(matches!(store.load().unwrap(), MemorySettings::None));
    assert!(!path.exists());
    store
        .save(hindsight(HINDSIGHT_CLOUD_URL, true, Some("test-secret")))
        .unwrap();
    let reloaded = MemorySettingsStore::new(&path);
    let saved = reloaded
        .save(hindsight(HINDSIGHT_CLOUD_URL, true, None))
        .unwrap();
    assert!(
        matches!(saved, MemorySettings::Hindsight { api_key: Some(ref key), .. } if key == "test-secret")
    );
    // Private response has no secret-bearing field.
    let response = toml::to_string(&saved.response()).unwrap();
    assert!(!response.contains("test-secret"));
    assert!(response.contains("api_key_configured = true"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    store.save(MemorySettings::None).unwrap();
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("test-secret")
    );
    assert!(matches!(reloaded.load().unwrap(), MemorySettings::None));
}

#[test]
fn self_hosted_optional_key_can_be_cleared_and_never_moves_to_another_host() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemorySettingsStore::new(dir.path().join("memory.toml"));
    store
        .save(hindsight("http://localhost:8888", false, Some("secret")))
        .unwrap();
    let moved = store
        .save(hindsight("https://other.example", false, None))
        .unwrap();
    assert!(matches!(
        moved,
        MemorySettings::Hindsight { api_key: None, .. }
    ));
    store
        .save(hindsight("https://other.example", false, Some("secret")))
        .unwrap();
    let cleared = store
        .save(hindsight("https://other.example", false, Some("")))
        .unwrap();
    assert!(matches!(
        cleared,
        MemorySettings::Hindsight { api_key: None, .. }
    ));
}

#[test]
fn invalid_settings_leave_saved_state_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemorySettingsStore::new(dir.path().join("memory.toml"));
    store
        .save(hindsight(HINDSIGHT_CLOUD_URL, true, Some("secret")))
        .unwrap();
    for base in [
        "file:///tmp/memory",
        "https://user:password@example.com",
        "https://example.com?token=secret",
        "https://example.com#secret",
    ] {
        assert!(store.save(hindsight(base, false, None)).is_err());
    }
    assert!(
        store
            .save(hindsight(HINDSIGHT_CLOUD_URL, true, Some("")))
            .is_err()
    );
    assert!(
        store
            .save(hindsight("https://other.example", true, Some("secret")))
            .is_err()
    );
    assert!(
        matches!(store.load().unwrap(), MemorySettings::Hindsight { api_key: Some(key), .. } if key == "secret")
    );
}

#[test]
fn corrupt_file_and_write_failure_do_not_leak_credentials_or_report_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.toml");
    std::fs::write(&path, "api_key = super-secret-invalid-toml").unwrap();
    let store = MemorySettingsStore::new(&path);
    assert!(
        !store
            .load()
            .err()
            .unwrap()
            .to_string()
            .contains("super-secret")
    );
    let unwritable = MemorySettingsStore::new(path.join("memory.toml"));
    assert!(unwritable.save(MemorySettings::None).is_err());
}

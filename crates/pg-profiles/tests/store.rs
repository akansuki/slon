use std::sync::{Arc, Mutex};

use pg_core::{ErrorKind, SslMode};
use pg_profiles::{
    KeyringSecrets, MemorySecrets, ProfileInput, ProfileStore, SecretError, SecretStore,
};
use tempfile::TempDir;

/// Lets a test keep a handle on the secrets the store owns.
#[derive(Clone, Default)]
struct Shared(Arc<MemorySecrets>);
impl SecretStore for Shared {
    fn get(&self, k: &str) -> Result<Option<String>, SecretError> {
        self.0.get(k)
    }
    fn set(&self, k: &str, v: &str) -> Result<(), SecretError> {
        self.0.set(k, v)
    }
    fn delete(&self, k: &str) -> Result<(), SecretError> {
        self.0.delete(k)
    }
}

/// A credential store that always fails.
struct Broken;
impl SecretStore for Broken {
    fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
        Err(SecretError("store locked".into()))
    }
    fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
        Err(SecretError("store locked".into()))
    }
    fn delete(&self, _: &str) -> Result<(), SecretError> {
        Err(SecretError("store locked".into()))
    }
}

/// Fails `set` on demand, to prove the order of operations.
struct Flaky {
    inner: MemorySecrets,
    fail_set: Mutex<bool>,
}

impl SecretStore for Flaky {
    fn get(&self, k: &str) -> Result<Option<String>, SecretError> {
        self.inner.get(k)
    }
    fn set(&self, k: &str, v: &str) -> Result<(), SecretError> {
        if *self.fail_set.lock().unwrap() {
            return Err(SecretError("denied".into()));
        }
        self.inner.set(k, v)
    }
    fn delete(&self, k: &str) -> Result<(), SecretError> {
        self.inner.delete(k)
    }
}

fn input(name: &str, password: Option<&str>) -> ProfileInput {
    ProfileInput {
        id: None,
        name: name.into(),
        host: "db.example.com".into(),
        port: 5432,
        database: "app".into(),
        user: "alice".into(),
        ssl_mode: SslMode::Require,
        ssl_root_cert: None,
        password: password.map(str::to_string),
        forget_password: false,
    }
}

fn fixture() -> (TempDir, Shared, ProfileStore) {
    let dir = TempDir::new().unwrap();
    let secrets = Shared::default();
    let store = ProfileStore::new(dir.path().join("profiles.json"), Box::new(secrets.clone()));
    (dir, secrets, store)
}

#[test]
fn empty_when_no_file() {
    let (_d, _s, store) = fixture();
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn password_never_reaches_the_file() {
    let (dir, secrets, store) = fixture();
    let p = store.save(input("prod", Some("hunter2-s3cret"))).unwrap();
    assert!(p.has_password);
    let text = std::fs::read_to_string(dir.path().join("profiles.json")).unwrap();
    assert!(!text.contains("hunter2-s3cret"), "{text}");
    assert!(text.contains("db.example.com"));
    assert_eq!(secrets.0.len(), 1);
    assert_eq!(
        store.saved_password(&p.id).unwrap().as_deref(),
        Some("hunter2-s3cret")
    );
}

#[test]
fn survives_a_new_store_instance() {
    let (dir, secrets, store) = fixture();
    let p = store.save(input("prod", Some("pw"))).unwrap();
    drop(store);
    let again = ProfileStore::new(dir.path().join("profiles.json"), Box::new(secrets));
    let list = again.list().unwrap();
    assert_eq!(list, vec![p.clone()]);
    assert_eq!(again.saved_password(&p.id).unwrap().as_deref(), Some("pw"));
}

#[test]
fn update_keeps_replaces_or_forgets_the_password() {
    let (_d, secrets, store) = fixture();
    let p = store.save(input("prod", Some("one"))).unwrap();

    // Keep: no password given.
    let mut i = input("prod renamed", None);
    i.id = Some(p.id.clone());
    let kept = store.save(i).unwrap();
    assert_eq!(kept.id, p.id);
    assert_eq!(kept.name, "prod renamed");
    assert!(kept.has_password);
    assert_eq!(store.saved_password(&p.id).unwrap().as_deref(), Some("one"));

    // Keep: empty string behaves like none.
    let mut i = input("prod renamed", Some(""));
    i.id = Some(p.id.clone());
    store.save(i).unwrap();
    assert_eq!(store.saved_password(&p.id).unwrap().as_deref(), Some("one"));

    // Replace.
    let mut i = input("prod renamed", Some("two"));
    i.id = Some(p.id.clone());
    store.save(i).unwrap();
    assert_eq!(store.saved_password(&p.id).unwrap().as_deref(), Some("two"));
    assert_eq!(secrets.0.len(), 1);

    // Forget.
    let mut i = input("prod renamed", None);
    i.id = Some(p.id.clone());
    i.forget_password = true;
    let f = store.save(i).unwrap();
    assert!(!f.has_password);
    assert_eq!(store.saved_password(&p.id).unwrap(), None);
    assert!(secrets.0.is_empty());
}

#[test]
fn unicode_and_odd_passwords_roundtrip() {
    let (_d, _s, store) = fixture();
    for pw in [
        "pässwörd-日本語-🔑",
        "  spaces  ",
        "quote\"and\\back\nnewline",
    ] {
        let p = store.save(input(pw, Some(pw))).unwrap();
        assert_eq!(store.saved_password(&p.id).unwrap().as_deref(), Some(pw));
    }
}

#[test]
fn delete_removes_profile_and_secret() {
    let (_d, secrets, store) = fixture();
    let a = store.save(input("a", Some("pw"))).unwrap();
    let b = store.save(input("b", None)).unwrap();
    store.delete(&a.id).unwrap();
    let list = store.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, b.id);
    assert!(secrets.0.is_empty());
    assert_eq!(store.delete(&a.id).unwrap_err().kind, ErrorKind::NotFound);
}

#[test]
fn saved_password_cases() {
    let (_d, secrets, store) = fixture();
    let none = store.save(input("nopw", None)).unwrap();
    assert_eq!(store.saved_password(&none.id).unwrap(), None);

    let with = store.save(input("pw", Some("x"))).unwrap();
    // Removed behind our back (e.g. in Credential Manager).
    secrets
        .0
        .delete(&format!("connection:{}", with.id))
        .unwrap();
    let err = store.saved_password(&with.id).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Storage);
    assert!(err.message.contains("Enter it again"));

    assert_eq!(
        store.saved_password("nope").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn saved_password_only_released_for_the_saved_target() {
    let (_d, _s, store) = fixture();
    let p = store.save(input("a", Some("pw"))).unwrap();
    let get = |h: &str, port: u16, u: &str| store.password_for_connect(&p.id, h, port, u).unwrap();
    assert_eq!(get("db.example.com", 5432, "alice").as_deref(), Some("pw"));
    assert_eq!(
        get(" db.example.com ", 5432, " alice").as_deref(),
        Some("pw")
    );
    assert_eq!(get("evil.example.com", 5432, "alice"), None);
    assert_eq!(get("db.example.com", 5433, "alice"), None);
    assert_eq!(get("db.example.com", 5432, "other"), None);
    assert_eq!(
        store
            .password_for_connect("missing", "db.example.com", 5432, "alice")
            .unwrap(),
        None
    );
}

#[test]
fn ssl_mode_and_root_cert_are_stored_and_trimmed() {
    let (dir, _s, store) = fixture();
    let mut i = input("tls", None);
    i.ssl_mode = SslMode::VerifyFull;
    i.ssl_root_cert = Some("  C:\\certs\\rds-ca.pem  ".into());
    let p = store.save(i).unwrap();
    assert_eq!(p.ssl_mode, SslMode::VerifyFull);
    assert_eq!(p.ssl_root_cert.as_deref(), Some("C:\\certs\\rds-ca.pem"));
    let text = std::fs::read_to_string(dir.path().join("profiles.json")).unwrap();
    assert!(text.contains("\"sslMode\": \"verify-full\""), "{text}");
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["sslRootCert"], "C:\\certs\\rds-ca.pem");

    // An empty path clears it.
    let mut i = input("tls", None);
    i.id = Some(p.id.clone());
    i.ssl_mode = SslMode::VerifyCa;
    i.ssl_root_cert = Some("   ".into());
    let q = store.save(i).unwrap();
    assert_eq!((q.ssl_mode, q.ssl_root_cert), (SslMode::VerifyCa, None));
}

#[test]
fn old_profiles_without_new_fields_still_load() {
    let (dir, _s, store) = fixture();
    std::fs::write(
        dir.path().join("profiles.json"),
        r#"{"version":1,"profiles":[{"id":"a","name":"old","host":"h","port":5432,"database":"d","user":"u","sslMode":"prefer"}]}"#,
    )
    .unwrap();
    let list = store.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].ssl_root_cert, None);
    assert!(!list[0].has_password);
}

#[test]
fn validation() {
    let (_d, _s, store) = fixture();
    let mut i = input("x", None);
    i.host = "  ".into();
    assert!(store.save(i).is_err());
    let mut i = input("x", None);
    i.user = "".into();
    assert!(store.save(i).is_err());
    let mut i = input("x", None);
    i.port = 0;
    assert!(store.save(i).is_err());
    let mut i = input("x", None);
    i.id = Some("missing".into());
    assert_eq!(store.save(i).unwrap_err().kind, ErrorKind::NotFound);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn default_name_and_trimming() {
    let (_d, _s, store) = fixture();
    let mut i = input("   ", None);
    i.host = "  h  ".into();
    let p = store.save(i).unwrap();
    assert_eq!(p.name, "alice@h/app");
    assert_eq!(p.host, "h");
    let mut i = input("", None);
    i.database = "".into();
    assert_eq!(store.save(i).unwrap().name, "alice@db.example.com/alice");
}

#[test]
fn sorted_by_last_use_then_name() {
    let (_d, _s, store) = fixture();
    let b = store.save(input("Bravo", None)).unwrap();
    let _a = store.save(input("alpha", None)).unwrap();
    let c = store.save(input("Charlie", None)).unwrap();
    let names = |s: &ProfileStore| {
        s.list()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&store), ["alpha", "Bravo", "Charlie"]);
    store.touch(&c.id).unwrap();
    assert_eq!(names(&store), ["Charlie", "alpha", "Bravo"]);
    // touching keeps settings, an unknown id is harmless
    store.touch("unknown").unwrap();
    store.touch(&b.id).unwrap();
    assert_eq!(store.list().unwrap().len(), 3);
}

#[test]
fn touch_survives_edit() {
    let (_d, _s, store) = fixture();
    let p = store.save(input("a", None)).unwrap();
    store.touch(&p.id).unwrap();
    let mut i = input("a2", None);
    i.id = Some(p.id.clone());
    let q = store.save(i).unwrap();
    assert!(q.last_used_at.is_some());
}

#[test]
fn damaged_file_is_reported_and_left_alone() {
    let (dir, _s, store) = fixture();
    let path = dir.path().join("profiles.json");
    std::fs::write(&path, "{ this is not json").unwrap();
    assert_eq!(store.list().unwrap_err().kind, ErrorKind::Storage);
    assert_eq!(
        store.save(input("x", Some("pw"))).unwrap_err().kind,
        ErrorKind::Storage
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{ this is not json"
    );
}

#[test]
fn newer_file_version_is_refused() {
    let (dir, _s, store) = fixture();
    let path = dir.path().join("profiles.json");
    std::fs::write(&path, r#"{"version":99,"profiles":[]}"#).unwrap();
    assert!(store.list().unwrap_err().message.contains("newer version"));
}

#[test]
fn broken_secret_store_saves_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.json");
    let store = ProfileStore::new(&path, Box::new(Broken));
    let err = store.save(input("x", Some("pw"))).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Storage);
    assert!(err.message.contains("store locked"));
    assert!(!path.exists());
    // Without a password the credential store is not needed at all.
    assert!(store.save(input("y", None)).is_ok());
}

#[test]
fn file_failure_rolls_the_secret_back() {
    let dir = TempDir::new().unwrap();
    // The "folder" for the profile file is a regular file, so writing must fail.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let secrets = Shared::default();
    let store = ProfileStore::new(blocker.join("profiles.json"), Box::new(secrets.clone()));
    let err = store.save(input("x", Some("pw"))).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Storage);
    assert!(
        secrets.0.is_empty(),
        "secret must not outlive a failed save"
    );
}

#[test]
fn file_failure_restores_previous_password() {
    let (dir, secrets, store) = fixture();
    let p = store.save(input("a", Some("old"))).unwrap();
    // Make the next write fail: occupy the temp file's name with a directory.
    std::fs::create_dir(dir.path().join("profiles.json.tmp")).unwrap();
    let mut i = input("a", Some("new"));
    i.id = Some(p.id.clone());
    assert!(store.save(i).is_err());
    assert_eq!(
        secrets
            .0
            .get(&format!("connection:{}", p.id))
            .unwrap()
            .as_deref(),
        Some("old")
    );
    assert_eq!(store.saved_password(&p.id).unwrap().as_deref(), Some("old"));
}

#[test]
fn secret_failure_on_update_keeps_old_state() {
    let dir = TempDir::new().unwrap();
    let flaky = Flaky {
        inner: MemorySecrets::new(),
        fail_set: Mutex::new(false),
    };
    let flaky = Arc::new(flaky);
    struct Wrap(Arc<Flaky>);
    impl SecretStore for Wrap {
        fn get(&self, k: &str) -> Result<Option<String>, SecretError> {
            self.0.get(k)
        }
        fn set(&self, k: &str, v: &str) -> Result<(), SecretError> {
            self.0.set(k, v)
        }
        fn delete(&self, k: &str) -> Result<(), SecretError> {
            self.0.delete(k)
        }
    }
    let store = ProfileStore::new(
        dir.path().join("profiles.json"),
        Box::new(Wrap(flaky.clone())),
    );
    let p = store.save(input("a", Some("old"))).unwrap();
    *flaky.fail_set.lock().unwrap() = true;
    let mut i = input("renamed", Some("new"));
    i.id = Some(p.id.clone());
    assert!(store.save(i).is_err());
    let list = store.list().unwrap();
    assert_eq!(
        list[0].name, "a",
        "profile must not change when the password cannot be stored"
    );
    assert_eq!(store.saved_password(&p.id).unwrap().as_deref(), Some("old"));
}

#[test]
fn input_debug_does_not_leak_password() {
    let i = input("x", Some("topsecret"));
    assert!(!format!("{i:?}").contains("topsecret"));
}

#[test]
fn profile_json_shape_for_the_frontend() {
    let (_d, _s, store) = fixture();
    let p = store.save(input("x", Some("pw"))).unwrap();
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["sslMode"], "require");
    assert_eq!(v["hasPassword"], true);
    assert!(v.get("password").is_none());
    let inp: ProfileInput = serde_json::from_str(
        r#"{"id":null,"name":"n","host":"h","port":1,"database":"d","user":"u","password":"p","forgetPassword":true}"#,
    )
    .unwrap();
    assert!(inp.forget_password);
    assert_eq!(inp.ssl_mode, SslMode::Prefer);
}

/// Talks to the real credential store of this machine (Windows Credential Manager on
/// Windows). Run with: cargo test -p pg-profiles -- --ignored
#[test]
#[ignore = "touches the real system credential store"]
fn real_credential_store_roundtrip() {
    let s = KeyringSecrets::new("slon-test");
    let key = format!("connection:test-{}", std::process::id());
    assert_eq!(s.get(&key).unwrap(), None);
    s.set(&key, "pässwörd 🔑 123").unwrap();
    assert_eq!(s.get(&key).unwrap().as_deref(), Some("pässwörd 🔑 123"));
    s.set(&key, "second").unwrap();
    assert_eq!(s.get(&key).unwrap().as_deref(), Some("second"));
    s.delete(&key).unwrap();
    assert_eq!(s.get(&key).unwrap(), None);
    s.delete(&key).unwrap(); // deleting twice is fine
}

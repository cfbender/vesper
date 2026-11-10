use std::fs;
use vesper_core::secrets::{self, MAX_SECRET_FILE_BYTES};
use vesper_core::SecretRef;

fn file(name: &str) -> SecretRef {
    SecretRef::File(name.into())
}

fn message(r: anyhow::Result<String>) -> String {
    format!("{:#}", r.unwrap_err())
}

#[test]
fn resolves_env_and_files_trimming_one_newline() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("bot.token"), "abc123\n").unwrap();
    fs::write(d.path().join("crlf"), "x\r\n").unwrap();
    fs::write(d.path().join("two"), "y\n\n").unwrap();
    assert_eq!(
        secrets::resolve(&file("bot.token"), d.path()).unwrap(),
        "abc123"
    );
    assert_eq!(secrets::resolve(&file("crlf"), d.path()).unwrap(), "x");
    assert_eq!(secrets::resolve(&file("two"), d.path()).unwrap(), "y\n");
    std::env::set_var("VESPER_TEST_SECRET_A", "from-env");
    assert_eq!(
        secrets::resolve(&SecretRef::Env("VESPER_TEST_SECRET_A".into()), d.path()).unwrap(),
        "from-env"
    );
}

#[test]
fn rejects_traversal_and_bad_names() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("secrets");
    fs::create_dir(&root).unwrap();
    fs::write(parent.path().join("outside"), "TOPSECRET").unwrap();
    for name in [
        "../outside",
        "/etc/passwd",
        "a/b",
        ".hidden",
        "..",
        "",
        "-x",
        "a\0b",
    ] {
        let m = message(secrets::resolve(&file(name), &root));
        assert!(m.contains("invalid file name"), "{name:?}: {m}");
    }
    for name in ["", "1ABC", "A-B", "A B", "$(x)"] {
        let m = message(secrets::resolve(&SecretRef::Env(name.into()), &root));
        assert!(
            m.contains("invalid environment variable name"),
            "{name:?}: {m}"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlinks_must_stay_inside_the_secrets_root() {
    use std::os::unix::fs::symlink;
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("secrets");
    fs::create_dir_all(root.join("..data")).unwrap();
    fs::write(parent.path().join("outside"), "TOPSECRET").unwrap();
    symlink(parent.path().join("outside"), root.join("escape")).unwrap();
    let m = message(secrets::resolve(&file("escape"), &root));
    assert!(m.contains("outside the secrets directory"), "{m}");
    assert!(!m.contains("TOPSECRET"));
    // Kubernetes-style secret mounts link into a hidden directory in the root.
    fs::write(root.join("..data/token"), "inside\n").unwrap();
    symlink("..data/token", root.join("token")).unwrap();
    assert_eq!(secrets::resolve(&file("token"), &root).unwrap(), "inside");
    // A directory is not a secret.
    fs::create_dir(root.join("dir")).unwrap();
    assert!(message(secrets::resolve(&file("dir"), &root)).contains("not a regular file"));
}

#[test]
fn bounded_non_empty_utf8_and_errors_hide_content() {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join("big"),
        vec![b'A'; MAX_SECRET_FILE_BYTES as usize + 1],
    )
    .unwrap();
    fs::write(d.path().join("empty"), "\n").unwrap();
    fs::write(d.path().join("binary"), [0xff, 0xfe, b'S', b'E', b'C']).unwrap();
    assert!(message(secrets::resolve(&file("big"), d.path())).contains("exceeds"));
    assert!(message(secrets::resolve(&file("empty"), d.path())).contains("is empty"));
    let m = message(secrets::resolve(&file("binary"), d.path()));
    assert!(m.contains("not valid UTF-8") && !m.contains("SEC"), "{m}");
    let m = message(secrets::resolve(&file("missing"), d.path()));
    assert!(m.contains("secret file missing is not accessible"), "{m}");
    let m = message(secrets::resolve(
        &SecretRef::Env("VESPER_TEST_SECRET_UNSET".into()),
        d.path(),
    ));
    assert!(m.contains("VESPER_TEST_SECRET_UNSET is not set"), "{m}");
}

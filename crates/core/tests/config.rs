use std::fs;
use std::path::Path;
use vesper_core::config::{self, SaveError};
use vesper_core::{Check, CheckKind, Machine, SecretRef, Service};

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn err(root: &Path) -> String {
    format!("{:#}", config::load(root).unwrap_err())
}

fn save_err(e: anyhow::Error) -> SaveError {
    e.downcast_ref::<SaveError>()
        .unwrap_or_else(|| panic!("not a SaveError: {e:#}"))
        .clone()
}

/// Every file below `root` (excluding nothing), for "nothing else changed" checks.
fn snapshot(root: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.push((rel, fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
    out.sort();
    out
}

const SINGLE: &str = "\
# Vesper configuration
defaults:
  interval_seconds: 60
  timeout_seconds: 5
  stale_after_seconds: 180
network:
  allowed_cidrs: [192.168.1.0/24]
machines:
  jelly:
    name: Jelly   # the media box
    site: home
    host_service: host
    services:
      host:
        name: Host
        checks:
          ping:
            kind: tcp
            host: 192.168.1.10
            port: 22
      jellyfin:
        name: Jellyfin
        icon: jellyfin
        depends_on: [jelly/host]
        checks:
          web:
            kind: http
            url: http://192.168.1.10:8096/health
            body_contains: Healthy
            headers:
              X-Api-Key: {env: JELLYFIN_KEY}
          beat:
            kind: push
            token: {file: jellyfin-beat}
            expected_interval_seconds: 600
";

fn split_layout(root: &Path) {
    write(
        root,
        "_index.yaml",
        "defaults:\n  interval_seconds: 60\n  timeout_seconds: 5\n  stale_after_seconds: 180\n",
    );
    write(root, "network.yaml", "allowed_cidrs: [192.168.1.0/24]\n");
    write(
        root,
        "machines/jelly/_index.yaml",
        "name: Jelly   # the media box\nsite: home\nhost_service: host\n",
    );
    write(
        root,
        "machines/jelly/services/host.yaml",
        "name: Host\nchecks:\n  ping:\n    kind: tcp\n    host: 192.168.1.10\n    port: 22\n",
    );
    write(
        root,
        "machines/jelly/services/jellyfin/_index.yaml",
        "# Jellyfin\nname: Jellyfin\nicon: jellyfin\ndepends_on: [jelly/host]\n",
    );
    write(
        root,
        "machines/jelly/services/jellyfin/checks/web.yaml",
        "kind: http\nurl: http://192.168.1.10:8096/health  # LAN\nbody_contains: Healthy\nheaders:\n  X-Api-Key: {env: JELLYFIN_KEY}\n",
    );
    write(
        root,
        "machines/jelly/services/jellyfin/checks/_index.yaml",
        "beat:\n  kind: push\n  token: {file: jellyfin-beat}\n  expected_interval_seconds: 600\n",
    );
}

#[test]
fn single_file_and_split_layouts_assemble_identically() {
    let a = tempfile::tempdir().unwrap();
    write(a.path(), "_index.yaml", SINGLE);
    let b = tempfile::tempdir().unwrap();
    split_layout(b.path());
    let la = config::load(a.path()).unwrap();
    let lb = config::load(b.path()).unwrap();
    assert_eq!(la.config, lb.config);
    assert_ne!(la.revision, lb.revision);
    assert!(la.writable && lb.writable);

    let svc = &la.config.machines["jelly"].services["jellyfin"];
    assert_eq!(
        svc.checks["web"].headers["X-Api-Key"],
        SecretRef::Env("JELLYFIN_KEY".into())
    );
    assert_eq!(
        svc.checks["beat"].token,
        Some(SecretRef::File("jellyfin-beat".into()))
    );
    assert_eq!(svc.checks["beat"].kind, CheckKind::Push);
    assert_eq!(la.config.defaults.interval_seconds, 60);
    assert_eq!(
        la.config.defaults.failure_threshold, 3,
        "unset defaults keep built-in values"
    );

    assert_eq!(
        la.sources["machines/jelly/services/jellyfin"],
        "_index.yaml"
    );
    let s = &lb.sources;
    assert_eq!(s["network"], "network.yaml");
    assert_eq!(s["defaults"], "_index.yaml");
    assert_eq!(s["machines/jelly"], "machines/jelly/_index.yaml");
    assert_eq!(
        s["machines/jelly/services/host"],
        "machines/jelly/services/host.yaml"
    );
    assert_eq!(
        s["machines/jelly/services/host/checks/ping"],
        "machines/jelly/services/host.yaml"
    );
    assert_eq!(
        s["machines/jelly/services/jellyfin"],
        "machines/jelly/services/jellyfin/_index.yaml"
    );
    assert_eq!(
        s["machines/jelly/services/jellyfin/checks/web"],
        "machines/jelly/services/jellyfin/checks/web.yaml"
    );
    assert_eq!(
        s["machines/jelly/services/jellyfin/checks/beat"],
        "machines/jelly/services/jellyfin/checks/_index.yaml"
    );
}

#[test]
fn revision_is_stable_and_tracks_content() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let r1 = config::load(d.path()).unwrap().revision;
    assert_eq!(r1, config::load(d.path()).unwrap().revision);
    write(
        d.path(),
        "machines/jelly/_index.yaml",
        "name: Jelly\nsite: home\nhost_service: host\n",
    );
    let r2 = config::load(d.path()).unwrap().revision;
    assert_ne!(r1, r2);
    // Hidden files (editor swap files, VCS metadata) are ignored entirely.
    write(d.path(), ".git/config", "x");
    write(d.path(), "machines/.jelly.yaml.swp", "\0garbage");
    assert_eq!(config::load(d.path()).unwrap().revision, r2);
}

#[test]
fn layout_collisions_are_rejected() {
    let cases: &[(&[(&str, &str)], &str)] = &[
        (
            &[
                ("_index.yaml", "machines:\n  a: {}\n"),
                ("machines/b.yaml", "{}"),
            ],
            "`machines` is defined both in _index.yaml and by machines/",
        ),
        (
            &[
                ("machines/a.yaml", "{}"),
                ("machines/a/_index.yaml", "name: A\n"),
            ],
            "machines/a.yaml and machines/a/ both define `a`",
        ),
        (
            &[("machines.yaml", "a: {}\n"), ("machines/b.yaml", "{}")],
            "machines.yaml and machines/ both define `machines`",
        ),
        (
            &[
                ("machines/_index.yaml", "a:\n  name: A\n"),
                ("machines/a.yaml", "name: A\n"),
            ],
            "`a` is defined both in machines/_index.yaml and by machines/a.yaml",
        ),
        (
            &[
                ("machines/a/_index.yaml", "services:\n  s: {}\n"),
                ("machines/a/services/t.yaml", "{}"),
            ],
            "`services` is defined both in machines/a/_index.yaml",
        ),
        (
            &[
                (
                    "machines/a/services/s/_index.yaml",
                    "checks:\n  c: {kind: tcp, host: h, port: 1}\n",
                ),
                (
                    "machines/a/services/s/checks/d.yaml",
                    "kind: tcp\nhost: h\nport: 1\n",
                ),
            ],
            "`checks` is defined both in machines/a/services/s/_index.yaml",
        ),
        (
            &[("_index.yaml", "machines:\n  a: {}\n  a: {}\n")],
            "duplicate key `a` on lines 2 and 3",
        ),
        (
            &[("machines/a.yaml", "{}"), ("machines/A.yaml", "{}")],
            "differ only by case",
        ),
        (
            &[("machines/a/name.yaml", "A\n")],
            "only services may be split into files",
        ),
        (
            &[("machines/a/services/s/checks/c/_index.yaml", "{}")],
            "cannot be a directory",
        ),
        (&[("defaults/_index.yaml", "{}")], "cannot be a directory"),
        (&[("machines/bad.id.yaml", "{}")], "name is not a valid ID"),
        (
            &[("machines/_index/x.yaml", "{}")],
            "name is not a valid ID",
        ),
        (&[("machines/a.yml", "{}")], "use the .yaml extension"),
        (&[("README.md", "hi")], "unexpected file"),
        (
            &[("_index.yaml", "- a\n")],
            "_index.yaml: must contain a mapping",
        ),
        (
            &[("_index.yaml", "a: &x 1\nb: *x\n")],
            "anchors are not supported",
        ),
        (
            &[("_index.yaml", "telegram:\n  bot_token: !env TOKEN\n")],
            "tags are not supported",
        ),
        (
            &[("_index.yaml", "machines: {}\n---\nmachines: {}\n")],
            "multiple YAML documents",
        ),
        (
            &[("_index.yaml", "mahcines: {}\n")],
            "unknown field `mahcines`",
        ),
    ];
    for (files, expected) in cases {
        let d = tempfile::tempdir().unwrap();
        for (p, c) in *files {
            write(d.path(), p, c);
        }
        let e = err(d.path());
        assert!(
            e.contains(expected),
            "{files:?}: expected {expected:?} in {e:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlinks_and_escapes_are_rejected() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "evil.yaml", "name: Evil\n");

    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("machines")).unwrap();
    symlink(
        outside.path().join("evil.yaml"),
        d.path().join("machines/evil.yaml"),
    )
    .unwrap();
    assert!(err(d.path()).contains("symlinks are not allowed"));

    let d = tempfile::tempdir().unwrap();
    symlink(outside.path(), d.path().join("machines")).unwrap();
    assert!(err(d.path()).contains("symlinks are not allowed"));

    // The root path itself may be a symlink to the real directory.
    let real = tempfile::tempdir().unwrap();
    write(real.path(), "_index.yaml", SINGLE);
    let link_parent = tempfile::tempdir().unwrap();
    let link = link_parent.path().join("config");
    symlink(real.path(), &link).unwrap();
    assert!(config::load(&link).is_ok());
}

#[test]
fn size_and_depth_bounds() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        &format!("# {}\n", "x".repeat(1024 * 1024)),
    );
    assert!(err(d.path()).contains("larger than"));

    let d = tempfile::tempdir().unwrap();
    let deep = format!("machines: {}{}\n", "[".repeat(100), "]".repeat(100));
    write(d.path(), "_index.yaml", &deep);
    assert!(err(d.path()).contains("nesting deeper"));

    let d = tempfile::tempdir().unwrap();
    write(d.path(), "a/b/c/d/e/f/g/x.yaml", "{}");
    let e = err(d.path());
    assert!(
        e.contains("deeper than") || e.contains("unexpected entry"),
        "{e}"
    );
}

#[test]
fn validation_rejects_semantic_errors() {
    let base = |svc: &str| {
        format!("network:\n  allowed_cidrs: [10.0.0.0/8]\nmachines:\n  m:\n    services:\n      s:\n{svc}")
    };
    let cases = [
        ("        depends_on: [m/nope]\n", "unknown service `m/nope`"),
        ("        depends_on: [m/s]\n", "cannot depend on itself"),
        ("        maintenance:\n          - {start: tomorrow, end: '2026-01-01T00:00:00Z'}\n", "start: must be an RFC3339"),
        (
            "        maintenance:\n          - {start: '2026-01-02T00:00:00Z', end: '2026-01-01T00:00:00Z'}\n",
            "end must be after start",
        ),
        ("        checks:\n          c: {kind: http, url: 'http://user:pw@x.lan/'}\n", "credentials in URLs"),
        ("        checks:\n          c: {kind: http, url: 'ftp://x.lan/'}\n", "absolute http(s) URL"),
        ("        checks:\n          c: {kind: tcp, host: x.lan}\n", "port: required"),
        ("        checks:\n          c: {kind: push}\n", "push checks need a secret reference"),
        ("        checks:\n          c: {kind: tcp, host: x, port: 1, timeout_seconds: 60}\n", "timeout_seconds must not exceed"),
        (
            "        checks:\n          c: {kind: http, url: 'http://x/', headers: {X-K: {env: 'bad name'}}}\n",
            "env reference must name a variable",
        ),
        (
            "        checks:\n          c: {kind: push, token: {file: ../../etc/passwd}}\n",
            "plain file name inside the secrets directory",
        ),
        ("        checks:\n          c: {kind: http, url: 'http://x/', port: 80}\n", "host and port apply to tcp"),
        ("        checks:\n          c: {kind: http, url: 'http://x/', expected_status: 99}\n", "expected_status"),
        ("        icon: '<script>'\n", "icon"),
    ];
    for (svc, expected) in cases {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "_index.yaml", &base(svc));
        let e = err(d.path());
        assert!(e.contains(expected), "{svc}: expected {expected:?} in {e}");
    }
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        "network:\n  allowed_cidrs: [10.0.0.0/33]\n",
    );
    assert!(err(d.path()).contains("not a CIDR"));
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        "machines:\n  m:\n    host_service: nope\n",
    );
    assert!(err(d.path()).contains("no service `nope`"));
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        "machines:\n  m:\n    services:\n      a: {depends_on: [m/b]}\n      b: {depends_on: [m/c]}\n      c: {depends_on: [m/a]}\n",
    );
    assert!(err(d.path()).contains("dependency cycle"));
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        "defaults:\n  failure_threshold: 0\n",
    );
    assert!(err(d.path()).contains("failure_threshold"));
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", "telegram:\n  bot_token: {env: T}\n  chat_id: {env: C}\n  dashboard_url: 'https://u:p@vesper.lan'\n");
    assert!(err(d.path()).contains("must not contain credentials"));
}

#[test]
fn notification_channels_are_validated_against_their_kind() {
    let channels = |body: &str| {
        format!("notifications:\n  dashboard_url: https://vesper.lan\n  channels:\n{body}")
    };
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        &channels("    phone:\n      kind: ntfy\n      topic: {file: ntfy-topic}\n      priority: 5\n    mail:\n      kind: smtp\n      host: smtp.example.com\n      from: v@example.com\n      to: [a@example.com, b@example.com]\n      password: {env: SMTP_PASSWORD}\n"),
    );
    let loaded = config::load(d.path()).unwrap();
    let n = loaded.config.effective_notifications().unwrap();
    assert_eq!(n.channels["phone"].kind, "ntfy");
    assert_eq!(loaded.sources["notifications"], "_index.yaml");

    for (body, expected) in [
        ("    x:\n      kind: carrier_pigeon\n", "notifications.channels.x.kind: names no known channel kind"),
        ("    x:\n      kind: ntfy\n", "notifications.channels.x.topic: required"),
        ("    x:\n      kind: ntfy\n      topic: t\n      colour: red\n", "notifications.channels.x.colour: not a setting of kind ntfy"),
        ("    x:\n      kind: discord\n      webhook_url: https://discord.com/api/webhooks/1/abc\n", "notifications.channels.x.webhook_url: credentials must be secret references"),
        ("    x:\n      kind: ntfy\n      topic: t\n      priority: 9\n", "notifications.channels.x.priority: must be one of: 1, 2, 3, 4, 5"),
        ("    x:\n      kind: ntfy\n      topic: t\n      server_url: ftp://ntfy.lan\n", "notifications.channels.x.server_url: must be an absolute http(s) URL"),
        ("    x:\n      kind: smtp\n      host: h\n      from: f\n      to: a@b.c\n      port: many\n", "notifications.channels.x.port: must be an integer"),
        ("    x:\n      kind: ntfy\n      topic: t\n      access_token: {file: ../t}\n", "notifications.channels.x.access_token: file reference must be a plain file name"),
    ] {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "_index.yaml", &channels(body));
        let e = err(d.path());
        assert!(e.contains(expected), "{body}: {e}");
    }

    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        "telegram:\n  bot_token: {env: T}\n  chat_id: {env: C}\n  dashboard_url: https://v.lan\nnotifications:\n  dashboard_url: https://v.lan\n",
    );
    assert!(err(d.path()).contains("both cannot be set"));
}

#[test]
fn errors_never_echo_misplaced_secret_values() {
    let secret = "hunter2-SUPERSECRET";
    let layouts = [
        format!("machines:\n  m:\n    services:\n      s:\n        checks:\n          c:\n            kind: http\n            url: http://x/\n            headers:\n              Authorization: Bearer {secret}\n"),
        format!("telegram:\n  bot_token: {secret}\n  chat_id: {{env: C}}\n  dashboard_url: https://v.lan\n"),
        format!("telegram:\n  bot_token: {{env: {secret}}}\n  chat_id: {{env: C}}\n  dashboard_url: https://v.lan\n"),
        format!("telegram:\n  bot_token: {{value: {secret}}}\n  chat_id: {{env: C}}\n  dashboard_url: https://v.lan\n"),
        format!("machines:\n  m:\n    services:\n      s:\n        checks:\n          c:\n            kind: push\n            token: {{file: '{secret}/x'}}\n"),
        format!("machines:\n  m:\n    services:\n      s:\n        checks:\n          c:\n            kind: push\n            port: {secret}\n"),
        format!("machines:\n  '{secret} x': {{}}\n"),
        format!("notifications:\n  dashboard_url: https://v.lan\n  channels:\n    d:\n      kind: discord\n      webhook_url: https://discord.com/api/webhooks/1/{secret}\n"),
        format!("notifications:\n  dashboard_url: https://v.lan\n  channels:\n    d:\n      kind: ntfy\n      topic: t\n      bogus_{secret}: 1\n"),
        format!("notifications:\n  dashboard_url: https://v.lan\n  channels:\n    d:\n      kind: ntfy\n      topic: t\n      server_url: 'https://u:{secret}@ntfy.lan'\n"),
    ];
    for yaml in layouts {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "_index.yaml", &yaml);
        let e = config::load(d.path()).map(|_| ()).expect_err(&yaml);
        let e = format!("{e:#}");
        assert!(!e.contains("SUPERSECRET"), "leaked in: {e}");
    }
}

#[test]
fn secret_refs_use_the_same_mapping_form_in_yaml_and_json() {
    let check = Check {
        kind: CheckKind::Push,
        token: Some(SecretRef::File("beat".into())),
        ..Check::default()
    };
    let json = serde_json::to_value(&check).unwrap();
    assert_eq!(json["token"], serde_json::json!({ "file": "beat" }));
    let mut service = Service::default();
    service.checks.insert("c".into(), check.clone());
    let yaml = config::export_service(&service).unwrap();
    assert_eq!(
        yaml,
        "checks:\n  c:\n    kind: push\n    token: {file: beat}\n"
    );
    assert!(!yaml.contains('!'));
    // And the YAML round-trips through the loader.
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "machines/m/services/s.yaml", &yaml);
    let loaded = config::load(d.path()).unwrap();
    assert_eq!(loaded.config.machines["m"].services["s"], service);
}

fn loaded(root: &Path) -> config::LoadedConfig {
    config::load(root).unwrap()
}

#[test]
fn save_service_edits_in_place_and_preserves_comments() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    let l = loaded(d.path());
    let mut svc = l.config.machines["jelly"].services["jellyfin"].clone();
    svc.name = "Jellyfin Media".into();
    svc.paused = true;
    svc.checks.get_mut("web").unwrap().url = Some("https://jelly.lan/health".into());
    svc.checks.get_mut("beat").unwrap().body_contains = None;
    svc.checks.remove("beat");
    svc.checks.insert(
        "tcp".into(),
        Check {
            kind: CheckKind::Tcp,
            host: Some("192.168.1.10".into()),
            port: Some(8096),
            ..Check::default()
        },
    );
    let after = config::save_service(d.path(), &l.revision, "jelly", "jellyfin", &svc).unwrap();
    assert_eq!(after.config.machines["jelly"].services["jellyfin"], svc);
    assert_ne!(after.revision, l.revision);
    let text = read(d.path(), "_index.yaml");
    let expected = SINGLE
        .replace("        name: Jellyfin\n", "        name: Jellyfin Media\n")
        .replace(
            "            url: http://192.168.1.10:8096/health\n",
            "            url: https://jelly.lan/health\n",
        )
        .replace(
            "          beat:\n            kind: push\n            token: {file: jellyfin-beat}\n            expected_interval_seconds: 600\n",
            "          tcp:\n            kind: tcp\n            host: 192.168.1.10\n            port: 8096\n",
        )
        .replace("        depends_on: [jelly/host]\n", "        depends_on: [jelly/host]\n        paused: true\n");
    // `paused` is appended at the end of the service mapping, after checks.
    let expected = expected.replace(
        "        depends_on: [jelly/host]\n        paused: true\n",
        "        depends_on: [jelly/host]\n",
    ) + "        paused: true\n";
    assert_eq!(text, expected);
    assert!(
        text.starts_with("# Vesper configuration\n")
            && text.contains("name: Jelly   # the media box")
    );
}

#[test]
fn save_in_split_layout_touches_only_owning_files() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let before = snapshot(d.path());
    let l = loaded(d.path());
    let mut svc = l.config.machines["jelly"].services["jellyfin"].clone();
    svc.checks.get_mut("web").unwrap().body_contains = Some("OK".into());
    svc.checks.remove("beat");
    svc.checks.insert(
        "api".into(),
        Check {
            url: Some("http://192.168.1.10:8096/api".into()),
            ..Check::default()
        },
    );
    let after = config::save_service(d.path(), &l.revision, "jelly", "jellyfin", &svc).unwrap();
    assert_eq!(after.config.machines["jelly"].services["jellyfin"], svc);
    let now = snapshot(d.path());
    let changed: Vec<&str> = now
        .iter()
        .filter(|f| !before.contains(f))
        .map(|(p, _)| p.as_str())
        .chain(
            before
                .iter()
                .filter(|f| !now.iter().any(|n| n.0 == f.0))
                .map(|(p, _)| p.as_str()),
        )
        .collect();
    let mut changed = changed;
    changed.sort();
    assert_eq!(
        changed,
        vec![
            "machines/jelly/services/jellyfin/checks/_index.yaml",
            "machines/jelly/services/jellyfin/checks/api.yaml",
            "machines/jelly/services/jellyfin/checks/web.yaml",
        ]
    );
    assert_eq!(
        read(d.path(), "machines/jelly/services/jellyfin/checks/web.yaml"),
        "kind: http\nurl: http://192.168.1.10:8096/health  # LAN\nbody_contains: OK\nheaders:\n  X-Api-Key: {env: JELLYFIN_KEY}\n"
    );
    assert_eq!(
        read(d.path(), "machines/jelly/services/jellyfin/checks/api.yaml"),
        "kind: http\nurl: http://192.168.1.10:8096/api\n"
    );
    assert_eq!(
        read(
            d.path(),
            "machines/jelly/services/jellyfin/checks/_index.yaml"
        ),
        ""
    );
    assert_eq!(
        after.sources["machines/jelly/services/jellyfin/checks/api"],
        "machines/jelly/services/jellyfin/checks/api.yaml"
    );
}

#[test]
fn new_service_in_directory_collection_gets_its_own_file() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let l = loaded(d.path());
    let mut svc = Service {
        name: "Sonarr".into(),
        depends_on: vec!["jelly/host".into()],
        ..Service::default()
    };
    svc.checks.insert(
        "web".into(),
        Check {
            url: Some("http://192.168.1.10:8989/".into()),
            ..Check::default()
        },
    );
    let after = config::save_service(d.path(), &l.revision, "jelly", "sonarr", &svc).unwrap();
    assert_eq!(
        after.sources["machines/jelly/services/sonarr"],
        "machines/jelly/services/sonarr/_index.yaml"
    );
    assert_eq!(after.config.machines["jelly"].services["sonarr"], svc);
    // Sibling services include a directory, so the new one mirrors that layout.
    assert_eq!(
        read(d.path(), "machines/jelly/services/sonarr/_index.yaml"),
        "name: Sonarr\ndepends_on:\n  - jelly/host\n"
    );
    assert_eq!(
        read(d.path(), "machines/jelly/services/sonarr/checks/web.yaml"),
        "kind: http\nurl: http://192.168.1.10:8989/\n"
    );

    // A machine directory without any services yet gets services/<id>.yaml.
    let l = loaded(d.path());
    let m = Machine {
        name: "NAS".into(),
        ..Machine::default()
    };
    let l2 = config::save_machine(d.path(), &l.revision, "nas", &m).unwrap();
    assert_eq!(l2.sources["machines/nas"], "machines/nas/_index.yaml");
    let l3 = config::save_service(
        d.path(),
        &l2.revision,
        "nas",
        "smb",
        &Service {
            name: "SMB".into(),
            ..Service::default()
        },
    )
    .unwrap();
    assert_eq!(
        l3.sources["machines/nas/services/smb"],
        "machines/nas/services/smb.yaml"
    );
    assert_eq!(
        read(d.path(), "machines/nas/services/smb.yaml"),
        "name: SMB\n"
    );
}

#[test]
fn new_service_in_inline_mapping_is_appended() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    let l = loaded(d.path());
    let svc = Service {
        name: "Radarr".into(),
        description: "Movies: \"HD\"".into(),
        ..Service::default()
    };
    let after = config::save_service(d.path(), &l.revision, "jelly", "radarr", &svc).unwrap();
    assert_eq!(after.config.machines["jelly"].services["radarr"], svc);
    let text = read(d.path(), "_index.yaml");
    assert!(text.starts_with(SINGLE), "existing text preserved");
    assert_eq!(
        &text[SINGLE.len()..],
        "      radarr:\n        name: Radarr\n        description: \"Movies: \\\"HD\\\"\"\n"
    );
}

#[test]
fn revision_conflict_rejects_and_leaves_files_untouched() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    let l = loaded(d.path());
    write(
        d.path(),
        "_index.yaml",
        &SINGLE.replace("site: home", "site: office"),
    );
    let before = snapshot(d.path());
    let svc = Service::default();
    let e =
        save_err(config::save_service(d.path(), &l.revision, "jelly", "new", &svc).unwrap_err());
    let current = loaded(d.path()).revision;
    assert_eq!(
        e,
        SaveError::Conflict {
            current_revision: current
        }
    );
    assert_eq!(snapshot(d.path()), before);
    let e = save_err(
        config::save_machine(d.path(), "sha256:stale", "x", &Machine::default()).unwrap_err(),
    );
    assert!(matches!(e, SaveError::Conflict { .. }));
}

#[cfg(unix)]
#[test]
fn read_only_root_is_reported_and_refuses_saves() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let l = loaded(d.path());
    let result = config::save_service(d.path(), &l.revision, "jelly", "x", &Service::default());
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o755)).unwrap();
    if l.writable {
        // Running as root: permission bits do not apply.
        return;
    }
    assert_eq!(save_err(result.unwrap_err()), SaveError::ReadOnly);
}

#[test]
fn comment_loss_becomes_export_only() {
    let d = tempfile::tempdir().unwrap();
    let text = SINGLE.replace(
        "        depends_on: [jelly/host]\n",
        "        maintenance:\n          - start: 2026-01-01T00:00:00Z  # patch night\n            end: 2026-01-01T02:00:00Z\n            repeat_weekly: true\n          - start: 2026-02-01T00:00:00Z\n            end: 2026-02-01T02:00:00Z\n",
    );
    write(d.path(), "_index.yaml", &text);
    let l = loaded(d.path());
    let mut svc = l.config.machines["jelly"].services["jellyfin"].clone();
    assert_eq!(svc.maintenance.len(), 2);
    svc.maintenance.pop();
    let e = save_err(
        config::save_service(d.path(), &l.revision, "jelly", "jellyfin", &svc).unwrap_err(),
    );
    assert!(matches!(e, SaveError::ExportOnly { .. }), "{e:?}");
    assert!(e.to_string().contains("export the YAML"));
    assert_eq!(read(d.path(), "_index.yaml"), text, "nothing written");
    // The export still works for the user to apply by hand.
    let exported = config::export_service(&svc).unwrap();
    assert!(exported.starts_with("name: Jellyfin\n"), "{exported}");
    // Edits elsewhere in the same file remain possible.
    let mut svc = l.config.machines["jelly"].services["jellyfin"].clone();
    svc.description = "Media".into();
    config::save_service(d.path(), &l.revision, "jelly", "jellyfin", &svc).unwrap();
    assert!(read(d.path(), "_index.yaml").contains("# patch night"));
}

#[test]
fn removing_split_checks_deletes_their_files_only() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let l = loaded(d.path());
    let mut svc = l.config.machines["jelly"].services["jellyfin"].clone();
    svc.checks.clear();
    let after = config::save_service(d.path(), &l.revision, "jelly", "jellyfin", &svc).unwrap();
    assert!(after.config.machines["jelly"].services["jellyfin"]
        .checks
        .is_empty());
    assert!(!d
        .path()
        .join("machines/jelly/services/jellyfin/checks/web.yaml")
        .exists());
    assert_eq!(
        read(d.path(), "machines/jelly/services/jellyfin/_index.yaml"),
        "# Jellyfin\nname: Jellyfin\nicon: jellyfin\ndepends_on: [jelly/host]\n"
    );
}

#[test]
fn removing_every_inline_check_leaves_an_empty_mapping() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    let l = loaded(d.path());
    let mut svc = l.config.machines["jelly"].services["host"].clone();
    svc.checks.clear();
    let after = config::save_service(d.path(), &l.revision, "jelly", "host", &svc).unwrap();
    assert!(after.config.machines["jelly"].services["host"]
        .checks
        .is_empty());
    assert!(read(d.path(), "_index.yaml")
        .contains("        name: Host\n        checks: {}\n      jellyfin:\n"));
}

#[test]
fn save_machine_updates_fields_but_never_services() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let l = loaded(d.path());
    let mut m = l.config.machines["jelly"].clone();
    m.site = "rack".into();
    m.description = "Media server".into();
    m.services.clear();
    let after = config::save_machine(d.path(), &l.revision, "jelly", &m).unwrap();
    assert_eq!(after.config.machines["jelly"].site, "rack");
    assert_eq!(
        after.config.machines["jelly"].services,
        l.config.machines["jelly"].services
    );
    assert_eq!(
        read(d.path(), "machines/jelly/_index.yaml"),
        "name: Jelly   # the media box\nsite: rack\nhost_service: host\ndescription: Media server\n"
    );
    // Sending services that differ is refused rather than mass-editing.
    let mut m2 = after.config.machines["jelly"].clone();
    m2.services.remove("host");
    m2.host_service = None;
    let e = config::save_machine(d.path(), &after.revision, "jelly", &m2).unwrap_err();
    assert!(format!("{e:#}").contains("save each service separately"));
    // Identical services are accepted.
    let mut m3 = after.config.machines["jelly"].clone();
    m3.name = "Jelly 2".into();
    config::save_machine(d.path(), &after.revision, "jelly", &m3).unwrap();
}

#[test]
fn new_machine_with_services_follows_collection_layout() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "machines/a.yaml", "name: A\n");
    let l = loaded(d.path());
    let mut m = Machine {
        name: "B".into(),
        ..Machine::default()
    };
    m.services.insert(
        "s".into(),
        Service {
            name: "S".into(),
            ..Service::default()
        },
    );
    let after = config::save_machine(d.path(), &l.revision, "b", &m).unwrap();
    assert_eq!(after.config.machines["b"], m);
    assert_eq!(
        read(d.path(), "machines/b.yaml"),
        "name: B\nservices:\n  s:\n    name: S\n"
    );

    // An empty root gets a machines/ directory.
    let e = tempfile::tempdir().unwrap();
    let l = loaded(e.path());
    assert!(l.config.machines.is_empty());
    let after = config::save_machine(e.path(), &l.revision, "c", &m).unwrap();
    assert_eq!(after.sources["machines/c"], "machines/c.yaml");
}

#[test]
fn invalid_edits_and_ids_are_rejected_before_writing() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    let l = loaded(d.path());
    let before = snapshot(d.path());
    let bad = Service {
        depends_on: vec!["jelly/missing".into()],
        ..Service::default()
    };
    assert!(config::save_service(d.path(), &l.revision, "jelly", "x", &bad).is_err());
    assert!(
        config::save_service(d.path(), &l.revision, "jelly", "../x", &Service::default()).is_err()
    );
    assert!(config::save_service(
        d.path(),
        &l.revision,
        "jelly",
        "_index",
        &Service::default()
    )
    .is_err());
    assert!(config::save_service(d.path(), &l.revision, "nope", "x", &Service::default()).is_err());
    assert!(
        config::save_machine(d.path(), &l.revision, &"m".repeat(81), &Machine::default()).is_err()
    );
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn saves_leave_no_temporary_files_and_keep_modes() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", SINGLE);
    fs::set_permissions(
        d.path().join("_index.yaml"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let l = loaded(d.path());
    let mut svc = l.config.machines["jelly"].services["host"].clone();
    svc.name = "Host 2".into();
    config::save_service(d.path(), &l.revision, "jelly", "host", &svc).unwrap();
    let names: Vec<String> = fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["_index.yaml"]);
    let mode = fs::metadata(d.path().join("_index.yaml"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn repeated_saves_converge_without_spurious_edits() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let l = loaded(d.path());
    let before = snapshot(d.path());
    // Saving the unchanged service writes nothing (defaults are not added).
    let svc = l.config.machines["jelly"].services["jellyfin"].clone();
    let after = config::save_service(d.path(), &l.revision, "jelly", "jellyfin", &svc).unwrap();
    assert_eq!(after.revision, l.revision);
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn export_config_round_trips() {
    let d = tempfile::tempdir().unwrap();
    split_layout(d.path());
    let l = loaded(d.path());
    let text = config::export_config(&l.config).unwrap();
    let e = tempfile::tempdir().unwrap();
    write(e.path(), "_index.yaml", &text);
    assert_eq!(loaded(e.path()).config, l.config);
}

#[test]
fn json_formatted_yaml_is_edited_in_place_and_stays_json() {
    for text in ["{\"machines\": {\"m\": {\"name\": \"M\", \"services\": {\"s\": {\"name\": \"S\", \"checks\": {\"web\": {\"kind\": \"http\", \"url\": \"http://10.0.0.1/\"}, \"old\": {\"kind\": \"tcp\", \"host\": \"h\", \"port\": 1}}}}}}}\n", "{\n  \"machines\": {\n    \"m\": {\n      \"name\": \"M\",\n      \"services\": {\n        \"s\": {\n          \"name\": \"S\",\n          \"checks\": {\n            \"web\": {\n              \"kind\": \"http\",\n              \"url\": \"http://10.0.0.1/\"\n            },\n            \"old\": {\n              \"kind\": \"tcp\",\n              \"host\": \"h\",\n              \"port\": 1\n            }\n          }\n        }\n      }\n    }\n  }\n}\n"] {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "_index.yaml", text);
        let l = loaded(d.path());
        let mut svc = l.config.machines["m"].services["s"].clone();
        svc.name = "Renamed: \"S\"".into();
        svc.checks.remove("old");
        svc.checks.get_mut("web").unwrap().port = None;
        svc.checks.get_mut("web").unwrap().headers.insert("X-Key".into(), SecretRef::Env("KEY".into()));
        svc.paused = true;
        let after = config::save_service(d.path(), &l.revision, "m", "s", &svc).unwrap();
        assert_eq!(after.config.machines["m"].services["s"], svc);
        let written = read(d.path(), "_index.yaml");
        let json: serde_json::Value = serde_json::from_str(&written).unwrap_or_else(|e| panic!("{e}: {written}"));
        assert_eq!(json["machines"]["m"]["services"]["s"]["name"], "Renamed: \"S\"");
        assert_eq!(json["machines"]["m"]["services"]["s"]["checks"]["web"]["headers"]["X-Key"]["env"], "KEY");
        assert!(json["machines"]["m"]["services"]["s"]["checks"].get("old").is_none());
        // Only the edited regions changed: the machine prefix is intact.
        let prefix = &text[..text.find("\"services\"").unwrap()];
        assert!(written.starts_with(prefix));
    }
}

#[test]
fn probe_controlled_settings_are_validated() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "_index.yaml",
        "network:\n  private_ca_file: /etc/ssl/ca.pem\n",
    );
    assert!(err(d.path()).contains("private_ca_file: must be a plain file name"));
    for h in ["Transfer-Encoding", "connection", "TE", "Upgrade"] {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "_index.yaml",
            &format!("machines:\n  m:\n    services:\n      s:\n        checks:\n          c: {{kind: http, url: 'http://x/', headers: {{{h}: {{env: K}}}}}}\n"),
        );
        assert!(err(d.path()).contains("cannot be set"), "{h}");
    }
}

#[test]
fn edit_files_writes_only_changes_that_produce_the_planned_config() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "machines/m/_index.yaml", "name: M\n");
    let before = snapshot(r);
    let web = |planned: &config::LoadedConfig, name: &str| {
        let mut next = planned.config.clone();
        next.machines.get_mut("m").unwrap().services.insert(
            "web".into(),
            Service {
                name: name.into(),
                ..Service::default()
            },
        );
        next
    };
    let change = vec![config::FileChange::Write {
        path: "machines/m/services/web.yaml".into(),
        content: "# generated\nname: Web\n".into(),
    }];

    // A plan whose files would not produce its stated configuration, an
    // unsafe path, and a dry run all leave the directory untouched.
    let e =
        config::edit_files(r, false, |l, _, _| Ok((change.clone(), web(l, "Other")))).unwrap_err();
    assert!(e
        .to_string()
        .contains("do not produce the planned configuration"));
    for path in [
        "machines/../x.yaml",
        "machines/m/.hidden.yaml",
        "machines/m/web.txt",
    ] {
        let bad = vec![config::FileChange::Write {
            path: path.into(),
            content: String::new(),
        }];
        assert!(config::edit_files(r, false, |l, _, _| Ok((bad, l.config.clone()))).is_err());
    }
    let dry = config::edit_files(r, true, |l, _, _| Ok((change.clone(), web(l, "Web")))).unwrap();
    assert_eq!(dry.config.machines["m"].services["web"].name, "Web");
    assert_eq!(snapshot(r), before);

    let saved =
        config::edit_files(r, false, |l, _, _| Ok((change.clone(), web(l, "Web")))).unwrap();
    assert_eq!(
        read(r, "machines/m/services/web.yaml"),
        "# generated\nname: Web\n"
    );
    assert_eq!(saved, config::load(r).unwrap());
    config::edit_files(r, false, |l, files, _| {
        assert!(files.contains_key("machines/m/services/web.yaml"));
        let mut next = l.config.clone();
        next.machines.get_mut("m").unwrap().services.clear();
        Ok((
            vec![config::FileChange::Delete {
                path: "machines/m/services/web.yaml".into(),
            }],
            next,
        ))
    })
    .unwrap();
    assert!(config::load(r).unwrap().config.machines["m"]
        .services
        .is_empty());
}

#[test]
fn whole_file_edits_reject_oversized_files_before_writing_even_in_dry_runs() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", "{}\n");
    let before = snapshot(d.path());
    for dry_run in [true, false] {
        let error = config::edit_files(d.path(), dry_run, |loaded, _, _| {
            Ok((
                vec![config::FileChange::Write {
                    path: "_index.yaml".into(),
                    content: format!("# {}\n{{}}\n", "x".repeat(1024 * 1024)),
                }],
                loaded.config.clone(),
            ))
        })
        .unwrap_err();
        assert!(error.to_string().contains("larger than 1048576 bytes"));
        assert_eq!(snapshot(d.path()), before);
        config::load(d.path()).unwrap();
    }
    // Exactly the limit is still readable, not conservatively rejected.
    let content = format!("# {}\n{{}}\n", "x".repeat(1024 * 1024 - 6));
    assert_eq!(content.len(), 1024 * 1024);
    config::edit_files(d.path(), false, |loaded, _, _| {
        Ok((
            vec![config::FileChange::Write {
                path: "_index.yaml".into(),
                content,
            }],
            loaded.config.clone(),
        ))
    })
    .unwrap();
    config::load(d.path()).unwrap();
}

#[test]
fn whole_file_edits_enforce_tree_limits_before_creating_directories() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "_index.yaml", "{}\n");
    let before = snapshot(d.path());
    let cases = [
        (
            (0..16)
                .map(|i| config::FileChange::Write {
                    path: format!("machines/m{i}.yaml"),
                    content: format!("# {}\n", "x".repeat(1024 * 1024 - 3)),
                })
                .collect::<Vec<_>>(),
            "exceeds 16777216 bytes in total",
        ),
        (
            (0..10_000)
                .map(|i| config::FileChange::Write {
                    path: format!("machines/m{i}.yaml"),
                    content: "{}\n".into(),
                })
                .collect(),
            "more than 10000 entries",
        ),
        (
            vec![config::FileChange::Write {
                path: "a/b/c/d/e/f/g/_index.yaml".into(),
                content: "{}\n".into(),
            }],
            "directories nest deeper than 6 levels",
        ),
    ];
    for (changes, message) in cases {
        for dry_run in [true, false] {
            let e = config::edit_files(d.path(), dry_run, |l, _, _| {
                Ok((changes.clone(), l.config.clone()))
            })
            .unwrap_err();
            assert!(e.to_string().contains(message), "{e:#}");
            assert_eq!(snapshot(d.path()), before);
            assert!(!d.path().join("machines").exists());
            assert!(!d.path().join("a").exists());
        }
    }
}

#[test]
fn labels_file_sits_under_every_other_file_and_saves_write_over_it() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        config::LABELS_FILE,
        "network: {allowed_cidrs: [10.0.0.0/8], max_concurrent: 4}\nmachines:\n  m: {name: From labels, site: Rack}\n  only: {name: Only labels}\n",
    );
    write(r, "network.yaml", "allowed_cidrs: [10.1.0.0/16]\n");
    write(r, "machines/m/services/s.yaml", "name: S\n");
    let l = loaded(r);
    assert_eq!(l.config.network.allowed_cidrs, ["10.1.0.0/16"]);
    assert_eq!(l.config.network.max_concurrent, 4);
    assert_eq!(l.config.machines["m"].name, "From labels");
    assert_eq!(l.sources["network"], "network.yaml");
    assert_eq!(l.sources["machines/m"], "machines/m/");
    assert_eq!(l.sources["machines/only"], config::LABELS_FILE);

    // Renaming a machine named by labels writes the name to its own file,
    // which then wins; the labels file is never edited.
    let labels = read(r, config::LABELS_FILE);
    let m = Machine {
        name: "Renamed".into(),
        ..l.config.machines["m"].clone()
    };
    let after = config::save_machine(r, &l.revision, "m", &m).unwrap();
    assert_eq!(read(r, "machines/m/_index.yaml"), "name: Renamed\n");
    assert_eq!(after.config.machines["m"].site, "Rack");
    assert_eq!(read(r, config::LABELS_FILE), labels);

    write(
        r,
        config::LABELS_FILE,
        "machines:\n  m:\n    services: {}\n",
    );
    assert!(err(r).contains("_labels.yaml: machines.m.services: services cannot be set here"));
}

//! End-to-end reconciliation against a fake Docker Engine API on a Unix
//! socket and a real configuration directory.

use bytes::Bytes;
use http_body_util::Full;
use hyper::{service::service_fn, Response};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vesper_core::config;
use vesper_discover::{docker::Endpoint, reconcile, reconcile_once, Settings};

type Labels<'a> = &'a [(&'a str, &'a str)];

/// A Docker API that serves the current container list and records requests.
#[derive(Clone)]
struct FakeDocker {
    socket: PathBuf,
    containers: Arc<Mutex<serde_json::Value>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeDocker {
    fn start(dir: &Path, name: &str) -> FakeDocker {
        let socket = dir.join(format!("{name}.sock"));
        let fake = FakeDocker {
            socket: socket.clone(),
            containers: Arc::new(Mutex::new(json!([]))),
            requests: Arc::default(),
        };
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let state = fake.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let state = state.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                        let state = state.clone();
                        async move {
                            state.requests.lock().unwrap().push(format!(
                                "{} {}",
                                req.method(),
                                req.uri()
                            ));
                            let body = state.containers.lock().unwrap().to_string();
                            Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from(body))))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        fake
    }

    /// Sets the containers as (name, state, labels).
    fn set(&self, containers: &[(&str, &str, Labels)]) {
        let list: Vec<_> = containers
            .iter()
            .map(|(name, state, labels)| {
                let labels: serde_json::Map<String, serde_json::Value> = labels
                    .iter()
                    .map(|(k, v)| (k.to_string(), json!(v)))
                    .collect();
                json!({"Id": format!("{name:0>64}"), "Names": [format!("/{name}")], "State": state, "Labels": labels})
            })
            .collect();
        *self.containers.lock().unwrap() = json!(list);
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint::Unix(self.socket.clone())
    }
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn settings(root: &Path, sources: Vec<(&str, Endpoint)>, remove_after: Option<u64>) -> Settings {
    Settings {
        config_dir: root.to_path_buf(),
        sources: sources
            .into_iter()
            .map(|(m, e)| (m.to_string(), e))
            .collect(),
        interval: Duration::from_secs(30),
        remove_after: remove_after.map(Duration::from_secs),
        allowed_secrets: Default::default(),
        own_container: None,
    }
}

/// A configuration with a hand-written service on a directory-backed machine.
fn inventory() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    write(
        r,
        "_index.yaml",
        "network:\n  allowed_cidrs: [10.0.0.0/8]\n",
    );
    write(r, "machines/docker1/_index.yaml", "name: Docker 1\n");
    write(
        r,
        "machines/docker1/services/host.yaml",
        "# mine\nname: Host\nchecks:\n  ssh: {kind: tcp, host: 10.0.0.2, port: 22}\n",
    );
    write(r, "machines/docker2/_index.yaml", "name: Docker 2\n");
    root
}

const APP: &[(&str, &str)] = &[
    ("vesper.app.name", "App"),
    ("vesper.app.depends_on", "docker1/host"),
    ("vesper.app.checks.web.url", "http://10.0.0.2:8080/health"),
    ("vesper.app.checks.port.port", "8080"),
    ("vesper.app.checks.port.host", "10.0.0.2"),
    ("com.docker.compose.service", "app"),
];

#[tokio::test]
async fn creates_updates_and_never_touches_hand_written_files() {
    let root = inventory();
    let r = root.path();
    let sockets = tempfile::tempdir().unwrap();
    let docker = FakeDocker::start(sockets.path(), "d1");
    let s = settings(r, vec![("docker1", docker.endpoint())], Some(3600));
    let hand_written = read(r, "machines/docker1/services/host.yaml");

    // A stopped container still declares its service, and labels that
    // collide with the hand-written service are ignored.
    docker.set(&[
        ("app", "exited", APP),
        (
            "other",
            "running",
            &[("vesper.host.checks.web.url", "http://10.0.0.9/")],
        ),
    ]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.report.errors.is_empty(), "{:?}", out.report.errors);
    assert_eq!(
        out.report.changes,
        ["created docker1/app (machines/docker1/services/app.yaml)"]
    );
    assert!(out.report.warnings[0].contains("docker1/host: defined by hand"));
    assert_eq!(read(r, "machines/docker1/services/host.yaml"), hand_written);
    let loaded = config::load(r).unwrap();
    let app = &loaded.config.machines["docker1"].services["app"];
    assert_eq!(app.depends_on, ["docker1/host"]);
    assert_eq!(
        app.checks.keys().collect::<Vec<_>>(),
        ["port", "web"],
        "check IDs come from labels, so history keys stay stable"
    );
    assert_eq!(
        docker.requests.lock().unwrap().as_slice(),
        ["GET /containers/json?all=true"]
    );

    // Unchanged labels write nothing: the revision stays the same.
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.changes.is_empty() && out.report.changes.is_empty());
    assert_eq!(config::load(r).unwrap().revision, loaded.revision);

    // A label change updates the generated file in place.
    let mut labels = APP.to_vec();
    labels[0] = ("vesper.app.name", "App v2");
    docker.set(&[("app", "running", &labels)]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(
        out.report.changes,
        ["updated docker1/app (machines/docker1/services/app.yaml)"]
    );
    assert_eq!(
        config::load(r).unwrap().config.machines["docker1"].services["app"].name,
        "App v2"
    );

    // After a person edits the generated YAML, discovery leaves it alone,
    // even when the labels change again or disappear.
    let path = "machines/docker1/services/app.yaml";
    let edited = read(r, path).replace("name: App v2", "name: Mine");
    write(r, path, &edited);
    labels[0] = ("vesper.app.name", "App v3");
    docker.set(&[("app", "running", &labels)]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.changes.is_empty());
    assert!(out.report.warnings[0].contains("was edited by hand"));
    docker.set(&[]);
    let s0 = settings(r, vec![("docker1", docker.endpoint())], Some(0));
    assert!(reconcile_once(&s0, false).await.unwrap().changes.is_empty());
    assert_eq!(read(r, path), edited);
}

#[tokio::test]
async fn removal_waits_for_a_grace_period_and_never_follows_errors() {
    let root = inventory();
    let r = root.path();
    let sockets = tempfile::tempdir().unwrap();
    let tmp = sockets.path();
    let d1 = FakeDocker::start(tmp, "r1");
    let d2 = FakeDocker::start(tmp, "r2");
    let s = settings(
        r,
        vec![("docker1", d1.endpoint()), ("docker2", d2.endpoint())],
        Some(3600),
    );
    d1.set(&[("app", "running", APP)]);
    d2.set(&[(
        "db",
        "running",
        &[
            ("vesper.db.checks.pg.host", "10.0.0.3"),
            ("vesper.db.checks.pg.port", "5432"),
        ],
    )]);
    reconcile_once(&s, false).await.unwrap();
    let app = "machines/docker1/services/app.yaml";
    let db = "machines/docker2/services/db.yaml";
    let created = read(r, app);

    // Docker unavailable on docker1 (even with immediate removal) changes
    // nothing there, while docker2 is still reconciled.
    let gone = settings(
        r,
        vec![
            ("docker1", Endpoint::Unix(tmp.join("missing.sock"))),
            ("docker2", d2.endpoint()),
        ],
        Some(0),
    );
    d2.set(&[]);
    let out = reconcile_once(&gone, false).await.unwrap();
    assert!(out.report.errors[0].starts_with("docker1: cannot read Docker"));
    assert!(out.report.errors[0].contains("left its discovered services unchanged (1)"));
    assert_eq!(read(r, app), created);
    assert!(!r.join(db).exists(), "docker2 removed immediately");

    // A malformed label on the machine blocks removals there.
    d1.set(&[("x", "running", &[("vesper.bad id.name", "x")])]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(out.report.errors.len(), 1, "{:?}", out.report.errors);
    assert_eq!(read(r, app), created);

    // A valid service ID with an invalid field must also block removals,
    // including an already-due removal; fixing the labels permits it again.
    let immediate = settings(r, vec![("docker1", d1.endpoint())], Some(0));
    d1.set(&[("x", "running", &[("vesper.other.checks.web.port", "bad")])]);
    let out = reconcile_once(&immediate, false).await.unwrap();
    assert!(!out.report.errors.is_empty());
    assert_eq!(read(r, app), created);

    // Once the container is really gone, the service stays monitored and is
    // marked; a configuration reload sees no change.
    let before = config::load(r).unwrap();
    d1.set(&[]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.report.changes[0].contains("will be removed at"));
    let marked = read(r, app);
    assert!(marked.contains("missing-since="));
    assert_eq!(config::load(r).unwrap().config, before.config);
    assert!(reconcile_once(&s, false).await.unwrap().changes.is_empty());

    // Still missing after the grace period: removed.
    let sources = vec![reconcile::Source {
        machine: "docker1".into(),
        scan: Ok(Default::default()),
        allowed_secrets: vec![],
    }];
    let later = chrono::Utc::now() + chrono::Duration::seconds(3601);
    let loaded = config::edit_files(r, false, |loaded, files, preview| {
        let plan = reconcile::plan(
            loaded,
            files,
            preview,
            &sources,
            later,
            Some(Duration::from_secs(3600)),
        );
        assert!(plan.report.changes[0].starts_with("removed docker1/app"));
        Ok((plan.changes, plan.config))
    })
    .unwrap();
    assert!(!loaded.config.machines["docker1"]
        .services
        .contains_key("app"));
    assert!(loaded.config.machines["docker1"]
        .services
        .contains_key("host"));

    // A container that comes back within the grace period is restored.
    d1.set(&[("app", "running", APP)]);
    reconcile_once(&s, false).await.unwrap();
    d1.set(&[]);
    reconcile_once(&s, false).await.unwrap();
    d1.set(&[("app", "running", APP)]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(
        out.report.changes,
        [format!("restored docker1/app ({app})")]
    );
    assert_eq!(read(r, app), created);
}

#[tokio::test]
async fn invalid_services_are_isolated_and_dependencies_resolve_in_one_scan() {
    let root = inventory();
    let r = root.path();
    let sockets = tempfile::tempdir().unwrap();
    let docker = FakeDocker::start(sockets.path(), "v");
    let s = settings(r, vec![("docker1", docker.endpoint())], Some(3600));
    docker.set(&[
        // `alpha` depends on `zeta`, which sorts after it.
        (
            "a",
            "running",
            &[("vesper.alpha.depends_on", "docker1/zeta")],
        ),
        (
            "z",
            "running",
            &[("vesper.zeta.checks.web.url", "http://10.0.0.4/")],
        ),
        (
            "bad",
            "running",
            &[("vesper.bad.depends_on", "docker1/nothing")],
        ),
        (
            "far",
            "running",
            &[
                ("vesper.far.checks.web.url", "http://192.168.1.1/"),
                ("vesper.far.checks.web.timeout_seconds", "999"),
            ],
        ),
    ]);
    let dry = reconcile_once(&s, true).await.unwrap();
    assert_eq!(dry.changes.len(), 2);
    assert!(
        !r.join("machines/docker1/services/alpha.yaml").exists(),
        "dry run writes nothing"
    );
    let out = reconcile_once(&s, false).await.unwrap();
    let services = &config::load(r).unwrap().config.machines["docker1"].services;
    assert!(services.contains_key("alpha") && services.contains_key("zeta"));
    assert!(!services.contains_key("bad") && !services.contains_key("far"));
    assert_eq!(out.report.errors.len(), 2, "{:?}", out.report.errors);
    assert!(out.report.errors[0]
        .starts_with("docker1/bad: machines.docker1.services.bad.depends_on: unknown service"));
    assert!(out.report.errors[0].ends_with("not created"));
    assert!(out.report.errors[1].contains("timeout_seconds"));
}

#[tokio::test]
async fn missing_machines_become_directories_but_file_machines_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    write(r, "machines/nas.yaml", "name: NAS\n");
    write(r, "machines/host/_index.yaml", "name: Host\n");
    let sockets = tempfile::tempdir().unwrap();
    let docker = FakeDocker::start(sockets.path(), "n");
    docker.set(&[("app", "running", &APP[2..])]);
    let s = settings(
        r,
        vec![("nas", docker.endpoint()), ("ghost", docker.endpoint())],
        Some(3600),
    );
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(out.report.errors.len(), 1, "{:?}", out.report.errors);
    assert!(out.report.errors[0].contains("nas: discovery writes one file per service, but this machine is defined in machines/nas.yaml"));
    assert_eq!(
        out.report.changes,
        ["created ghost/app (machines/ghost/services/app.yaml)"]
    );
    let loaded = config::load(r).unwrap();
    assert_eq!(loaded.sources["machines/ghost"], "machines/ghost/");
    assert_eq!(loaded.config.machines["ghost"].name, "");

    // Machines written inline cannot gain a directory.
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "_index.yaml",
        "machines:\n  nas: {name: NAS}\n",
    );
    let s = settings(root.path(), vec![("ghost", docker.endpoint())], Some(3600));
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.changes.is_empty());
    assert!(out.report.errors[0]
        .contains("ghost: no machine directory, and machines are defined in _index.yaml"));
}

#[tokio::test]
async fn secret_grants_are_exact_per_machine_and_revocation_pauses_managed_services() {
    use vesper_core::SecretRef;
    let root = inventory();
    let r = root.path();
    let sockets = tempfile::tempdir().unwrap();
    let docker = FakeDocker::start(sockets.path(), "secrets");
    let mut s = settings(
        r,
        vec![
            ("docker1", docker.endpoint()),
            ("docker2", docker.endpoint()),
        ],
        Some(0),
    );
    docker.set(&[(
        "app",
        "running",
        &[
            ("vesper.app.checks.web.url", "http://10.0.0.2/"),
            (
                "vesper.app.checks.web.headers.Authorization.file",
                "app-token",
            ),
            ("vesper.app.checks.beat.token.env", "BEAT_TOKEN"),
        ],
    )]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(out.report.errors.len(), 2);
    assert!(out.changes.is_empty(), "secrets are denied by default");
    s.allowed_secrets.insert(
        "docker1".into(),
        vec![
            SecretRef::Env("app-token".into()),
            SecretRef::Env("BEAT_TOKEN".into()),
        ],
    );
    assert!(
        reconcile_once(&s, false).await.unwrap().changes.is_empty(),
        "same name with another reference type is not a grant"
    );
    s.allowed_secrets.insert(
        "docker1".into(),
        vec![
            SecretRef::File("app-token".into()),
            SecretRef::Env("BEAT_TOKEN".into()),
        ],
    );
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(out.report.errors.len(), 1, "docker2 has no grant");
    let loaded = config::load(r).unwrap();
    assert!(!loaded.config.machines["docker1"].services["app"].paused);
    assert!(!loaded.config.machines["docker2"]
        .services
        .contains_key("app"));
    let path = "machines/docker1/services/app.yaml";
    let good = read(r, path);
    docker.set(&[(
        "app",
        "running",
        &[
            ("vesper.app.checks.web.url", "http://10.0.0.2/"),
            (
                "vesper.app.checks.web.headers.Authorization.file",
                "telegram-bot-token",
            ),
        ],
    )]);
    reconcile_once(&s, false).await.unwrap();
    assert_eq!(
        read(r, path),
        good,
        "a new ungranted reference cannot replace a safe existing check"
    );
    s.allowed_secrets.clear();
    s.sources[0].1 = Endpoint::Unix(sockets.path().join("unavailable.sock"));
    reconcile_once(&s, true).await.unwrap();
    assert_eq!(read(r, path), good, "dry-run does not pause on disk");
    reconcile_once(&s, false).await.unwrap();
    assert!(
        config::load(r).unwrap().config.machines["docker1"].services["app"].paused,
        "revocation applies even without Docker"
    );
    assert_eq!(
        read(r, "machines/docker1/services/host.yaml"),
        "# mine\nname: Host\nchecks:\n  ssh: {kind: tcp, host: 10.0.0.2, port: 22}\n"
    );
    // Safe replacement labels may resume the service without restoring grants.
    s.sources[0].1 = docker.endpoint();
    docker.set(&[(
        "app",
        "running",
        &[("vesper.app.checks.web.url", "http://10.0.0.2/")],
    )]);
    reconcile_once(&s, false).await.unwrap();
    let loaded = config::load(r).unwrap();
    let app = &loaded.config.machines["docker1"].services["app"];
    assert!(!app.paused);
    assert!(app.checks["web"].headers.is_empty());
}

#[tokio::test]
async fn oversized_discovery_output_leaves_the_inventory_unchanged() {
    let root = inventory();
    let r = root.path();
    let sockets = tempfile::tempdir().unwrap();
    let docker = FakeDocker::start(sockets.path(), "large");
    let s = settings(r, vec![("docker1", docker.endpoint())], None);
    let before = config::load(r).unwrap();
    let mut labels = Vec::new();
    for i in 0..1100 {
        labels.push((
            format!("vesper.app.checks.c{i}.url"),
            "http://10.0.0.2/".to_string(),
        ));
        labels.push((
            format!("vesper.app.checks.c{i}.body_contains"),
            "x".repeat(1000),
        ));
    }
    let borrowed: Vec<_> = labels
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    docker.set(&[("large", "running", &borrowed)]);
    for dry_run in [true, false] {
        let error = reconcile_once(&s, dry_run)
            .await
            .err()
            .expect("oversized file rejected");
        assert!(format!("{error:#}").contains("larger than 1048576 bytes"));
        assert_eq!(config::load(r).unwrap(), before);
        assert!(!r.join("machines/docker1/services/app.yaml").exists());
    }
}

#[tokio::test]
async fn vesper_labels_configure_an_empty_directory_and_files_take_precedence() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let sockets = tempfile::tempdir().unwrap();
    let docker = FakeDocker::start(sockets.path(), "d1");
    let mut s = settings(r, vec![("docker1", docker.endpoint())], Some(3600));
    s.own_container = Some(format!("{:0>64}", "vesper"));
    let own: &[(&str, &str)] = &[
        ("vesper.network.allowed_cidrs", "10.0.0.0/8, 192.168.1.0/24"),
        ("vesper.network.max_response_bytes", "2048"),
        ("vesper.defaults.interval_seconds", "60"),
        ("vesper.defaults.stale_after_seconds", "180"),
        ("vesper.machines.docker1.name", "Docker 1"),
        (
            "vesper.notifications.dashboard_url",
            "https://status.example.com",
        ),
        ("vesper.notifications.channels.phone.kind", "ntfy"),
        (
            "vesper.notifications.channels.phone.topic.file",
            "ntfy-topic",
        ),
        (
            "vesper.vesper.checks.web.url",
            "http://10.0.0.2:8080/healthz",
        ),
    ];
    let intruder: &[(&str, &str)] = &[
        ("vesper.network.allowed_cidrs", "0.0.0.0/0"),
        ("vesper.app.checks.web.url", "http://10.0.0.3/"),
    ];
    docker.set(&[("vesper", "running", own), ("app", "running", intruder)]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.report.errors.is_empty(), "{:?}", out.report.errors);
    assert_eq!(
        out.report.changes,
        [
            "created _labels.yaml from Vesper's own labels",
            "created docker1/app (machines/docker1/services/app.yaml)",
            "created docker1/vesper (machines/docker1/services/vesper.yaml)",
        ]
    );
    assert_eq!(out.report.warnings.len(), 1);
    assert!(out.report.warnings[0]
        .starts_with("docker1: container app: label `vesper.network.allowed_cidrs` sets Vesper's own configuration"));
    let labels = read(r, "_labels.yaml");
    assert!(
        labels.contains("allowed_cidrs:\n    - \"10.0.0.0/8\"\n    - \"192.168.1.0/24\"\n"),
        "{labels}"
    );
    assert!(!labels.contains("0.0.0.0/0") && !labels.contains("max_concurrent"));
    let loaded = config::load(r).unwrap();
    let c = &loaded.config;
    assert_eq!(c.network.allowed_cidrs, ["10.0.0.0/8", "192.168.1.0/24"]);
    assert_eq!(
        (c.network.max_response_bytes, c.network.max_concurrent),
        (2048, 16)
    );
    assert_eq!(
        (c.defaults.interval_seconds, c.defaults.stale_after_seconds),
        (60, 180)
    );
    assert_eq!(c.machines["docker1"].name, "Docker 1");
    assert_eq!(
        c.notifications.as_ref().unwrap().channels["phone"].kind,
        "ntfy"
    );
    assert_eq!(loaded.sources["network"], "_labels.yaml");
    assert_eq!(loaded.sources["machines/docker1"], "machines/docker1/");
    assert!(reconcile_once(&s, false).await.unwrap().changes.is_empty());

    // Hand-written values win field by field; the rest still comes from labels.
    write(
        r,
        "_index.yaml",
        "network:\n  allowed_cidrs: [10.0.0.0/24]\ndefaults:\n  interval_seconds: 45\n",
    );
    write(r, "machines/docker1/_index.yaml", "name: Docker host\n");
    let loaded = config::load(r).unwrap();
    let c = &loaded.config;
    assert_eq!(c.network.allowed_cidrs, ["10.0.0.0/24"]);
    assert_eq!(c.network.max_response_bytes, 2048);
    assert_eq!(
        (c.defaults.interval_seconds, c.defaults.stale_after_seconds),
        (45, 180)
    );
    assert_eq!(c.machines["docker1"].name, "Docker host");
    assert_eq!(loaded.sources["network"], "_index.yaml");
    assert!(reconcile_once(&s, false).await.unwrap().changes.is_empty());

    // Labels that leave the merged configuration invalid change nothing.
    let mut invalid = own.to_vec();
    invalid[3] = ("vesper.defaults.stale_after_seconds", "10");
    docker.set(&[
        ("vesper", "running", &invalid),
        ("app", "running", intruder),
    ]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.changes.is_empty());
    assert!(
        out.report.errors[0].starts_with(
            "Vesper's own labels: defaults: stale_after_seconds must be at least interval_seconds"
        ),
        "{:?}",
        out.report.errors
    );
    assert_eq!(read(r, "_labels.yaml"), labels);

    // Without configuration labels the generated file goes away.
    docker.set(&[
        ("vesper", "running", &own[8..]),
        ("app", "running", intruder),
    ]);
    let out = reconcile_once(&s, false).await.unwrap();
    assert_eq!(
        out.report.changes[0],
        "removed _labels.yaml from Vesper's own labels"
    );
    assert!(!r.join("_labels.yaml").exists());
    assert_eq!(
        config::load(r).unwrap().config.network.max_response_bytes,
        1_048_576
    );

    // A hand-edited file is left alone.
    docker.set(&[("vesper", "running", own), ("app", "running", intruder)]);
    reconcile_once(&s, false).await.unwrap();
    let edited = read(r, "_labels.yaml").replace("2048", "4096");
    write(r, "_labels.yaml", &edited);
    let out = reconcile_once(&s, false).await.unwrap();
    assert!(out.changes.is_empty());
    assert!(out.report.warnings[0].starts_with("_labels.yaml was edited by hand"));
    assert_eq!(read(r, "_labels.yaml"), edited);
}

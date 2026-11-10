use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Method, Request},
};
use std::{net::SocketAddr, sync::Arc};
use tempfile::TempDir;
use tokio::sync::{Mutex, Semaphore};
use tower::ServiceExt;
use vesper_engine::Engine;

const TOKEN: &str = "local-test-heartbeat-token-32-characters";
const OTHER_TOKEN: &str = "different-local-heartbeat-token-32-characters";
const ORIGIN: &str = "https://vesper.example.test";

fn fixture() -> (TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    let data_dir = root.path().join("data");
    let secrets_dir = root.path().join("secrets");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(data_dir.join("icons")).unwrap();
    std::fs::create_dir_all(&secrets_dir).unwrap();
    std::fs::write(secrets_dir.join("push"), TOKEN).unwrap();
    std::fs::write(secrets_dir.join("other"), OTHER_TOKEN).unwrap();
    std::fs::write(
        config_dir.join("_index.yaml"),
        r#"
machines:
  home:
    name: Home
    services:
      backup:
        name: Backups
        checks:
          daily:
            kind: push
            token: {file: push}
          other:
            kind: push
            token: {file: other}
"#,
    )
    .unwrap();
    let loaded = config::load(&config_dir).unwrap();
    let engine = Engine::open(
        &data_dir.join("vesper.db"),
        loaded.clone(),
        secrets_dir.clone(),
    )
    .unwrap();
    let app = App {
        engine: Arc::new(Mutex::new(engine)),
        loaded: Arc::new(Mutex::new(loaded)),
        config_gate: Arc::new(Mutex::new(())),
        draft_slots: Arc::new(Semaphore::new(4)),
        config_dir,
        data_dir,
        secrets_dir,
        static_dir: root.path().join("web"),
        config_read_only: false,
        security: security::Security {
            origin: ORIGIN.into(),
            trusted_proxies: vec!["10.70.0.2/32".parse().unwrap()],
            csrf: "test-csrf".into(),
            dev_mode: false,
        },
    };
    (root, app)
}

async fn request(
    app: &App,
    method: Method,
    path: &str,
    body: Value,
    trusted: bool,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::from(body.to_string())).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        if trusted {
            "10.70.0.2:1234"
        } else {
            "203.0.113.8:4321"
        }
        .parse::<SocketAddr>()
        .unwrap(),
    ));
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)}));
    (status, headers, value)
}

#[tokio::test]
async fn proxy_origin_and_csrf_are_independent_boundaries() {
    let (_root, app) = fixture();
    let (status, _, _) = request(&app, Method::GET, "/healthz", Value::Null, false, &[]).await;
    assert_eq!(status, StatusCode::OK);
    for path in ["/api/snapshot", "/api/config", "/api/csrf", "/"] {
        let (status, _, _) = request(
            &app,
            Method::GET,
            path,
            Value::Null,
            false,
            &[("x-forwarded-for", "10.70.0.2"), ("remote-user", "admin")],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, headers, config) =
        request(&app, Method::GET, "/api/config", Value::Null, true, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!config.to_string().contains(TOKEN));
    assert!(!headers.contains_key("access-control-allow-origin"));
    assert_eq!(headers["x-content-type-options"], "nosniff");
    let until = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    for headers in [
        vec![],
        vec![("origin", ORIGIN)],
        vec![
            ("origin", "https://evil.example"),
            ("x-vesper-csrf", "test-csrf"),
        ],
        vec![("origin", ORIGIN), ("x-vesper-csrf", "wrong")],
    ] {
        let (status, _, _) = request(
            &app,
            Method::POST,
            "/api/services/home/backup/silence",
            json!({"until":until}),
            true,
            &headers,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/services/home/backup/silence",
        json!({"until":until}),
        true,
        &[("origin", ORIGIN), ("x-vesper-csrf", "test-csrf")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let snapshot = app.engine.lock().await.snapshot().unwrap();
    assert!(snapshot.machines[0].services[0].silenced_until.is_some());
    assert_eq!(
        snapshot.machines[0].services[0].health,
        vesper_core::Health::Unknown
    );
}

#[tokio::test]
async fn heartbeat_tokens_only_update_their_check_and_never_admit_inventory() {
    let (_root, app) = fixture();
    let credential = format!("Bearer {TOKEN}");
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/heartbeat/home/backup/daily",
        json!({"healthy":true}),
        false,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/heartbeat/home/backup/daily",
        json!({"healthy":true}),
        false,
        &[("authorization", &credential)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/heartbeat/home/backup/other",
        json!({"healthy":true}),
        false,
        &[("authorization", &credential)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = request(
        &app,
        Method::GET,
        "/api/config",
        Value::Null,
        false,
        &[("authorization", &credential)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let snapshot = app.engine.lock().await.snapshot().unwrap();
    assert_eq!(
        snapshot.machines[0].services[0].checks[0].health,
        vesper_core::Health::Healthy
    );
    assert_eq!(
        snapshot.machines[0].services[0].checks[1].health,
        vesper_core::Health::Unknown
    );
}

#[tokio::test]
async fn failed_reload_and_revision_conflicts_preserve_active_inventory() {
    let (_root, app) = fixture();
    let original = std::fs::read(app.config_dir.join("_index.yaml")).unwrap();
    std::fs::write(app.config_dir.join("_index.yaml"), "machines: [invalid").unwrap();
    let (status, _, _) = request(
        &app,
        Method::POST,
        "/api/config/reload",
        json!({}),
        true,
        &[("origin", ORIGIN), ("x-vesper-csrf", "test-csrf")],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let snapshot = app.engine.lock().await.snapshot().unwrap();
    assert_eq!(snapshot.machines[0].services[0].name, "Backups");
    assert!(snapshot.config_error.is_some());
    std::fs::write(app.config_dir.join("_index.yaml"), &original).unwrap();
    reload(&app).await.unwrap_or_else(|e| panic!("{}", e.1));
    let mut service = app.loaded.lock().await.config.machines["home"].services["backup"].clone();
    service.name = "Should not save".into();
    let (status, _, _) = request(
        &app,
        Method::PUT,
        "/api/services/home/backup",
        json!({"revision":"outdated","service":service}),
        true,
        &[("origin", ORIGIN), ("x-vesper-csrf", "test-csrf")],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        std::fs::read(app.config_dir.join("_index.yaml")).unwrap(),
        original
    );
}

#[tokio::test]
async fn draft_tests_obey_network_policy_without_changing_monitoring() {
    let (_root, app) = fixture();
    for _ in 0..3 {
        let (status, _, body) = request(
            &app,
            Method::POST,
            "/api/draft/test",
            json!({"checks":{"local":{"kind":"http","url":"http://127.0.0.1:8080/"}}}),
            true,
            &[("origin", ORIGIN), ("x-vesper-csrf", "test-csrf")],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["results"]["local"]["healthy"], false);
    }
    let snapshot = app.engine.lock().await.snapshot().unwrap();
    assert!(snapshot.incidents.is_empty());
    assert!(snapshot.machines[0].services[0]
        .checks
        .iter()
        .all(|c| c.history.is_empty()));
}

#[test]
fn uploads_are_bounded_raster_images_not_active_content() {
    assert!(normalize_icon(
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#
    )
    .is_err());
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(300, 180)
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let normalized = normalize_icon(png.get_ref()).unwrap_or_else(|e| panic!("{}", e.1));
    let decoded = image::load_from_memory(&normalized).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 154));
    assert!(!valid_upload_name(&format!("{}é.png", "a".repeat(62))));
    assert!(!valid_upload_name("../../secrets.png"));
}

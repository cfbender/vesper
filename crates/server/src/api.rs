use crate::{security, App};
use axum::{
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use image::{ImageFormat, ImageReader, Limits};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Cursor};
use tower_http::services::{ServeDir, ServeFile};
use vesper_core::{config, Check, CheckKind, Config, Machine, Service, Snapshot};

pub struct ApiError(StatusCode, String);
impl ApiError {
    fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    pub fn public_message(&self) -> String {
        self.1.chars().take(300).collect()
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.public_message()}))).into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        let message = error.to_string();
        let status = match error.downcast_ref::<config::SaveError>() {
            Some(config::SaveError::Conflict { .. }) => StatusCode::CONFLICT,
            Some(config::SaveError::ReadOnly) => StatusCode::FORBIDDEN,
            Some(config::SaveError::ExportOnly { .. }) => StatusCode::UNPROCESSABLE_ENTITY,
            None => StatusCode::BAD_REQUEST,
        };
        Self(status, message)
    }
}
type ApiResult<T> = Result<T, ApiError>;

struct Input<T>(T);
impl<S: Send + Sync, T: serde::de::DeserializeOwned> FromRequest<S> for Input<T> {
    type Rejection = ApiError;
    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Json::<T>::from_request(request, state),
        )
        .await
        .map_err(|_| ApiError(StatusCode::REQUEST_TIMEOUT, "Request body timed out".into()))?
        .map_err(|error| {
            ApiError(
                error.status(),
                "Invalid JSON request body or unsupported fields".into(),
            )
        })?;
        Ok(Self(value))
    }
}

pub fn router(app: App) -> Router {
    let static_files = ServeDir::new(&app.static_dir)
        .not_found_service(ServeFile::new(app.static_dir.join("index.html")));
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/api/csrf", get(csrf))
        .route("/api/snapshot", get(snapshot))
        .route("/api/config", get(configuration))
        .route("/api/config/reload", post(reload_config))
        .route("/api/machines/{machine}", put(save_machine))
        .route("/api/services/{machine}/{service}", put(save_service))
        .route("/api/services/{machine}/{service}/recheck", post(recheck))
        .route("/api/services/{machine}/{service}/silence", post(silence))
        .route("/api/incidents/{id}/acknowledge", post(acknowledge))
        .route("/api/draft/test", post(test_draft))
        .route("/api/telegram/test", post(telegram_test))
        .route(
            "/api/heartbeat/{machine}/{service}/{check}",
            post(heartbeat),
        )
        .route("/api/icons", get(icons).post(upload_icon))
        .route("/icons/uploads/{filename}", get(uploaded_icon))
        .route(
            "/api/{*rest}",
            get(api_not_found)
                .post(api_not_found)
                .put(api_not_found)
                .delete(api_not_found),
        )
        .fallback_service(static_files)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            security::protect,
        ))
        .with_state(app)
}

async fn api_not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Unknown API route".into())
}
async fn csrf(State(app): State<App>) -> Json<Value> {
    Json(json!({"token": app.security.csrf}))
}
async fn snapshot(State(app): State<App>) -> ApiResult<Json<Snapshot>> {
    Ok(Json(app.engine.lock().await.snapshot()?))
}
async fn configuration(State(app): State<App>) -> Json<Value> {
    let loaded = app.loaded.lock().await;
    Json(
        json!({"revision":loaded.revision,"writable":loaded.writable,"defaults":loaded.config.defaults,"machines":loaded.config.machines,"sources":loaded.sources,"telegram_configured":loaded.config.telegram.is_some(),"network":loaded.config.network}),
    )
}

pub async fn reload(app: &App) -> ApiResult<()> {
    let _guard = app.config_gate.lock().await;
    let root = app.config_dir.clone();
    let mut loaded = tokio::task::spawn_blocking(move || config::load(&root))
        .await
        .map_err(|_| ApiError::bad("Configuration read interrupted"))??;
    loaded.writable &= !app.config_read_only;
    let mut current = app.loaded.lock().await;
    if current.revision != loaded.revision || current.writable != loaded.writable {
        app.engine.lock().await.replace_config(loaded.clone())?;
        *current = loaded;
    }
    app.engine.lock().await.set_config_error(None);
    Ok(())
}
async fn reload_config(State(app): State<App>) -> ApiResult<Json<Value>> {
    if let Err(error) = reload(&app).await {
        app.engine
            .lock()
            .await
            .set_config_error(Some(error.public_message()));
        return Err(error);
    }
    app.engine
        .lock()
        .await
        .audit("config.reload", "configuration")?;
    Ok(Json(
        json!({"message":"Configuration validated and loaded"}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveService {
    revision: String,
    service: Service,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveMachine {
    revision: String,
    machine: Machine,
}

async fn save_service(
    State(app): State<App>,
    Path((machine, service)): Path<(String, String)>,
    Input(body): Input<SaveService>,
) -> ApiResult<Json<Value>> {
    ids(&[&machine, &service])?;
    let _guard = app.config_gate.lock().await;
    if !app.loaded.lock().await.writable {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Configuration is read-only; export the draft YAML instead".into(),
        ));
    }
    let root = app.config_dir.clone();
    let target = format!("{machine}/{service}");
    let loaded = tokio::task::spawn_blocking(move || {
        config::save_service(&root, &body.revision, &machine, &service, &body.service)
    })
    .await
    .map_err(|_| ApiError::bad("Configuration save interrupted"))??;
    activate_saved(&app, loaded, "config.service.save", &target).await
}

async fn save_machine(
    State(app): State<App>,
    Path(machine): Path<String>,
    Input(body): Input<SaveMachine>,
) -> ApiResult<Json<Value>> {
    ids(&[&machine])?;
    let _guard = app.config_gate.lock().await;
    if !app.loaded.lock().await.writable {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Configuration is read-only; export the draft YAML instead".into(),
        ));
    }
    let root = app.config_dir.clone();
    let target = machine.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        config::save_machine(&root, &body.revision, &machine, &body.machine)
    })
    .await
    .map_err(|_| ApiError::bad("Configuration save interrupted"))??;
    activate_saved(&app, loaded, "config.machine.save", &target).await
}

async fn activate_saved(
    app: &App,
    loaded: config::LoadedConfig,
    action: &str,
    target: &str,
) -> ApiResult<Json<Value>> {
    let revision = loaded.revision.clone();
    app.engine.lock().await.replace_config(loaded.clone())?;
    app.engine.lock().await.set_config_error(None);
    *app.loaded.lock().await = loaded;
    app.engine.lock().await.audit(action, target)?;
    Ok(Json(
        json!({"revision":revision,"message":"Configuration saved and activated"}),
    ))
}

async fn recheck(
    State(app): State<App>,
    Path((machine, service)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ids(&[&machine, &service])?;
    app.engine
        .lock()
        .await
        .recheck(&format!("{machine}/{service}"))?;
    Ok(Json(
        json!({"message":"Active checks queued; push checks wait for their producer"}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Silence {
    until: Option<DateTime<Utc>>,
}
async fn silence(
    State(app): State<App>,
    Path((machine, service)): Path<(String, String)>,
    Input(body): Input<Silence>,
) -> ApiResult<Json<Value>> {
    ids(&[&machine, &service])?;
    if body
        .until
        .is_some_and(|until| until <= Utc::now() || until > Utc::now() + chrono::Duration::days(30))
    {
        return Err(ApiError::bad("Silence must end within the next 30 days"));
    }
    app.engine
        .lock()
        .await
        .silence(&format!("{machine}/{service}"), body.until)?;
    Ok(Json(
        json!({"message":"Notification silence updated; monitoring continues"}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Acknowledge {
    acknowledged: bool,
}
async fn acknowledge(
    State(app): State<App>,
    Path(id): Path<i64>,
    Input(body): Input<Acknowledge>,
) -> ApiResult<Json<Value>> {
    app.engine.lock().await.acknowledge(id, body.acknowledged)?;
    Ok(Json(
        json!({"message":"Acknowledgement updated; health is unchanged"}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    checks: BTreeMap<String, Check>,
}
async fn test_draft(State(app): State<App>, Input(body): Input<Draft>) -> ApiResult<Json<Value>> {
    let _permit = app.draft_slots.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Draft testing is busy; retry shortly".into(),
        )
    })?;
    if body.checks.is_empty() || body.checks.len() > 32 {
        return Err(ApiError::bad("Draft requires 1–32 checks"));
    }
    let active = app.engine.lock().await.config().clone();
    let service = Service {
        name: "Draft".into(),
        checks: body.checks.clone(),
        ..Default::default()
    };
    let draft = Config {
        defaults: active.defaults.clone(),
        network: active.network.clone(),
        machines: BTreeMap::from([(
            "draft".into(),
            Machine {
                name: "Draft".into(),
                services: BTreeMap::from([("draft".into(), service)]),
                ..Default::default()
            },
        )]),
        telegram: None,
    };
    config::validate(&draft)?;
    let mut results = BTreeMap::new();
    for (id, check) in body.checks {
        let result =
            vesper_probe::run(&check, &active.defaults, &active.network, &app.secrets_dir).await;
        results.insert(id, result);
    }
    Ok(Json(json!({"results":results})))
}

async fn telegram_test(State(app): State<App>) -> ApiResult<Json<Value>> {
    let message = vesper_engine::test_telegram(app.engine.clone()).await?;
    app.engine.lock().await.audit("telegram.test", "telegram")?;
    Ok(Json(json!({"message":message})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Heartbeat {
    healthy: bool,
}
async fn heartbeat(
    State(app): State<App>,
    Path((machine, service, check)): Path<(String, String, String)>,
    headers: HeaderMap,
    Input(body): Input<Heartbeat>,
) -> ApiResult<Json<Value>> {
    let unauthorized = || {
        ApiError(
            StatusCode::UNAUTHORIZED,
            "Invalid heartbeat credential".into(),
        )
    };
    if ids(&[&machine, &service, &check]).is_err() {
        return Err(unauthorized());
    }
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(unauthorized)?;
    let mut engine = app.engine.lock().await;
    let reference = engine
        .config()
        .machines
        .get(&machine)
        .and_then(|m| m.services.get(&service))
        .filter(|s| !s.paused)
        .and_then(|s| s.checks.get(&check))
        .filter(|c| c.kind == CheckKind::Push && c.enabled)
        .and_then(|c| c.token.as_ref())
        .ok_or_else(unauthorized)?;
    let token =
        vesper_core::secrets::resolve(reference, &app.secrets_dir).map_err(|_| unauthorized())?;
    if !security::equal_secret(supplied, &token) {
        return Err(unauthorized());
    }
    engine.heartbeat(&format!("{machine}/{service}/{check}"), body.healthy)?;
    Ok(Json(json!({"accepted":true})))
}

async fn icons(State(app): State<App>) -> ApiResult<Json<Value>> {
    let catalog = tokio::fs::read(app.static_dir.join("icons/catalog.json"))
        .await
        .unwrap_or_default();
    let mut icons = serde_json::from_slice::<Value>(&catalog)
        .ok()
        .and_then(|v| v.get("icons").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut entries = tokio::fs::read_dir(app.data_dir.join("icons"))
        .await
        .map_err(|_| ApiError::bad("Icon storage is unavailable"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| ApiError::bad("Icon storage is unavailable"))?
    {
        let filename = entry.file_name().to_string_lossy().into_owned();
        if valid_upload_name(&filename) {
            icons.push(json!({"id":format!("upload:{filename}"),"name":"Uploaded icon","url":format!("/icons/uploads/{filename}")}));
        }
    }
    Ok(Json(json!({"icons":icons})))
}
async fn upload_icon(State(app): State<App>, mut form: Multipart) -> ApiResult<Json<Value>> {
    let _permit = app.draft_slots.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Image processing is busy; retry shortly".into(),
        )
    })?;
    let field = tokio::time::timeout(std::time::Duration::from_secs(10), form.next_field())
        .await
        .map_err(|_| ApiError(StatusCode::REQUEST_TIMEOUT, "Upload timed out".into()))?
        .map_err(|_| ApiError::bad("Invalid multipart upload"))?
        .ok_or_else(|| ApiError::bad("Upload a file field"))?;
    if field.name() != Some("file") {
        return Err(ApiError::bad("Upload a file field"));
    }
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(10), field.bytes())
        .await
        .map_err(|_| ApiError(StatusCode::REQUEST_TIMEOUT, "Upload timed out".into()))?
        .map_err(|_| ApiError::bad("Icon upload exceeds the 2 MiB limit"))?;
    let png = tokio::task::spawn_blocking(move || normalize_icon(&bytes))
        .await
        .map_err(|_| ApiError::bad("Icon processing interrupted"))??;
    let digest = Sha256::digest(&png);
    let filename = format!("{:x}.png", digest);
    let destination = app.data_dir.join("icons").join(&filename);
    tokio::fs::write(destination, png)
        .await
        .map_err(|_| ApiError::bad("Icon storage is not writable"))?;
    app.engine.lock().await.audit("icon.upload", &filename)?;
    Ok(Json(
        json!({"id":format!("upload:{filename}"),"url":format!("/icons/uploads/{filename}")}),
    ))
}
fn normalize_icon(bytes: &[u8]) -> ApiResult<Vec<u8>> {
    let format = image::guess_format(bytes).map_err(|_| {
        ApiError::bad("Use a PNG, JPEG or WebP image; SVG uploads are not accepted")
    })?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(ApiError::bad("Use a PNG, JPEG or WebP image"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| ApiError::bad("Invalid image or dimensions exceed 2048×2048"))?
        .thumbnail(256, 256);
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| ApiError::bad("Image conversion failed"))?;
    Ok(output.into_inner())
}
fn valid_upload_name(filename: &str) -> bool {
    filename.len() == 68
        && filename.ends_with(".png")
        && filename
            .bytes()
            .take(64)
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
async fn uploaded_icon(
    State(app): State<App>,
    Path(filename): Path<String>,
) -> ApiResult<Response> {
    if !valid_upload_name(&filename) {
        return Err(ApiError(StatusCode::NOT_FOUND, "Icon not found".into()));
    }
    let bytes = tokio::fs::read(app.data_dir.join("icons").join(filename))
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Icon not found".into()))?;
    Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}
fn ids(ids: &[&str]) -> ApiResult<()> {
    if ids.iter().all(|id| security::valid_id(id)) {
        Ok(())
    } else {
        Err(ApiError::bad(
            "IDs must use 1–80 letters, digits, hyphens or underscores",
        ))
    }
}

#[cfg(test)]
mod tests;

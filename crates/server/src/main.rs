use anyhow::{bail, Context, Result};
use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, Semaphore};
use vesper_core::config;
use vesper_engine::Engine;

mod api;
mod security;

#[derive(Clone)]
pub struct App {
    engine: Arc<Mutex<Engine>>,
    loaded: Arc<Mutex<config::LoadedConfig>>,
    config_gate: Arc<Mutex<()>>,
    draft_slots: Arc<Semaphore>,
    config_dir: PathBuf,
    data_dir: PathBuf,
    secrets_dir: PathBuf,
    static_dir: PathBuf,
    config_read_only: bool,
    security: security::Security,
}

fn directory(name: &str, fallback: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.into())
}

fn healthcheck() -> Result<()> {
    use std::io::{Read, Write};
    let listen: SocketAddr = env::var("VESPER_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .context("VESPER_LISTEN must be an IP address and port")?;
    let address = SocketAddr::new(
        if listen.is_ipv6() {
            "::1".parse()?
        } else {
            "127.0.0.1".parse()?
        },
        listen.port(),
    );
    let timeout = std::time::Duration::from_secs(2);
    let mut socket = std::net::TcpStream::connect_timeout(&address, timeout)
        .context("Vesper is not listening")?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    socket.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = Vec::new();
    socket.take(128).read_to_end(&mut response)?;
    if !response.starts_with(b"HTTP/1.1 200 ") {
        bail!("Vesper liveness check failed");
    }
    println!("Vesper is listening");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();
    let command = env::args().nth(1).unwrap_or_else(|| "serve".into());
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        println!("vesper [serve|validate|export|healthcheck|discover]\nOptional Docker label discovery: set VESPER_DISCOVER_DOCKER; see `vesper discover --help`.\nConfiguration: VESPER_CONFIG_DIR (./config)\nData: VESPER_DATA_DIR (./data)\nSecrets: VESPER_SECRETS_DIR (./secrets)\nFrontend: VESPER_STATIC_DIR (./web/dist)\nServe requires VESPER_PUBLIC_URL and VESPER_TRUSTED_PROXIES, or explicit VESPER_DEV_MODE=true.\nSee docs/installation.md for the reverse proxy boundary and optional authentication.");
        return Ok(());
    }
    if command == "healthcheck" {
        return healthcheck();
    }
    if command == "discover" {
        // One reconciliation; `serve` reconciles continuously when enabled.
        let args: Vec<String> = env::args().skip(2).collect();
        return vesper_discover::main(&args).await;
    }
    let config_dir = directory("VESPER_CONFIG_DIR", "./config");
    let mut loaded = config::load(&config_dir).context("Configuration could not be loaded")?;
    match command.as_str() {
        "validate" => {
            let services: usize = loaded
                .config
                .machines
                .values()
                .map(|m| m.services.len())
                .sum();
            let checks: usize = loaded
                .config
                .machines
                .values()
                .flat_map(|m| m.services.values())
                .map(|s| s.checks.len())
                .sum();
            println!("Valid configuration: {} machines, {services} services, {checks} checks. Revision {}", loaded.config.machines.len(), loaded.revision);
            return Ok(());
        }
        "export" => {
            // Secret references only: this never calls secrets::resolve.
            print!("{}", config::export_config(&loaded.config)?);
            return Ok(());
        }
        "serve" => (),
        _ => bail!("Unknown command. Use vesper --help."),
    }
    let security = security::Security::from_env()?;
    let listen: SocketAddr = env::var("VESPER_LISTEN")
        .unwrap_or_else(|_| {
            if security.dev_mode {
                "127.0.0.1:8080"
            } else {
                "0.0.0.0:8080"
            }
            .into()
        })
        .parse()
        .context("VESPER_LISTEN must be an IP address and port")?;
    let data_dir = directory("VESPER_DATA_DIR", "./data");
    std::fs::create_dir_all(data_dir.join("icons"))?;
    let secrets_dir = directory("VESPER_SECRETS_DIR", "./secrets");
    let config_read_only = env::var("VESPER_CONFIG_READ_ONLY").as_deref() == Ok("true");
    let discovery = vesper_discover::Settings::from_env()?;
    if discovery.is_some() && config_read_only {
        bail!("VESPER_DISCOVER_DOCKER writes generated service files, so it needs writable configuration; unset VESPER_CONFIG_READ_ONLY or VESPER_DISCOVER_DOCKER");
    }
    if let Some(settings) = &discovery {
        // Apply revoked secret grants before any persisted check can run.
        let outcome = vesper_discover::reconcile_once(settings, false).await?;
        for line in outcome.report.changes {
            tracing::info!("discovery: {line}");
        }
        loaded = config::load(&config_dir)?;
    }
    loaded.writable &= !config_read_only;
    let engine = Arc::new(Mutex::new(Engine::open(
        &data_dir.join("vesper.db"),
        loaded.clone(),
        secrets_dir.clone(),
    )?));
    let app = App {
        engine: engine.clone(),
        loaded: Arc::new(Mutex::new(loaded)),
        config_gate: Arc::new(Mutex::new(())),
        draft_slots: Arc::new(Semaphore::new(4)),
        config_dir,
        data_dir,
        secrets_dir,
        static_dir: directory("VESPER_STATIC_DIR", "./web/dist"),
        config_read_only,
        security,
    };
    tokio::spawn(vesper_engine::run(engine));
    if let Some(settings) = discovery {
        eprintln!("WARNING: Docker label discovery is on ({}). Access to a Docker socket is root-equivalent on that host; see docs/discovery.md.", settings.describe());
        tokio::spawn(vesper_discover::run(vesper_discover::Settings {
            config_dir: app.config_dir.clone(),
            ..settings
        }));
    }
    let reload_app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3));
        loop {
            tick.tick().await;
            if let Err(error) = api::reload(&reload_app).await {
                reload_app
                    .engine
                    .lock()
                    .await
                    .set_config_error(Some(error.public_message()));
            }
        }
    });
    let router = api::router(app);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!("Vesper listening on {listen}.");
    eprintln!("WARNING: Vesper has no built-in authentication. Without authentication at your reverse proxy, anyone who can reach Vesper has full administrator access. Vesper cannot verify your proxy's authentication configuration.");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

#![allow(dead_code)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use vesper_core::config::LoadedConfig;
use vesper_core::*;
use vesper_engine::*;

pub struct ManualClock(StdMutex<DateTime<Utc>>);
impl ManualClock {
    pub fn advance(&self, seconds: i64) {
        *self.0.lock().unwrap() += Duration::seconds(seconds);
    }
    pub fn get(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}
impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

/// Returns the configured health per check ID and counts calls. A gate makes
/// probes block until released.
#[derive(Default)]
pub struct FakeProber {
    pub healthy: StdMutex<HashMap<String, bool>>,
    pub calls: StdMutex<HashMap<String, usize>>,
    pub gate: StdMutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub running: StdMutex<usize>,
    pub max_running: StdMutex<usize>,
}
impl FakeProber {
    pub fn set(&self, check_id: &str, healthy: bool) {
        self.healthy
            .lock()
            .unwrap()
            .insert(check_id.into(), healthy);
    }
    pub fn calls(&self, check_id: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .get(check_id)
            .copied()
            .unwrap_or(0)
    }
}
impl Prober for FakeProber {
    fn probe<'a>(
        &'a self,
        check_id: &'a str,
        _check: &'a Check,
        _defaults: &'a CheckDefaults,
        _network: &'a NetworkPolicy,
        _secrets_root: &'a Path,
    ) -> BoxFuture<'a, ProbeResult> {
        Box::pin(async move {
            *self
                .calls
                .lock()
                .unwrap()
                .entry(check_id.into())
                .or_default() += 1;
            {
                let mut r = self.running.lock().unwrap();
                *r += 1;
                let mut m = self.max_running.lock().unwrap();
                *m = (*m).max(*r);
            }
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            *self.running.lock().unwrap() -= 1;
            let healthy = self
                .healthy
                .lock()
                .unwrap()
                .get(check_id)
                .copied()
                .unwrap_or(true);
            ProbeResult {
                healthy,
                latency_ms: if healthy { 12 } else { 5000 },
                // Evidence must never reach Telegram.
                message: if healthy {
                    "HTTP 200".into()
                } else {
                    "HTTP 503 body-evidence-xyz".into()
                },
            }
        })
    }
}

/// Mock notification transport: records HTTP requests and replies with
/// scripted responses (default: Telegram's 200 ok).
#[derive(Default)]
pub struct FakeTelegram {
    pub sent: StdMutex<Vec<vesper_notify::HttpRequest>>,
    pub replies: StdMutex<VecDeque<vesper_notify::Response>>,
}
impl FakeTelegram {
    pub fn reply(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back(vesper_notify::Response {
                status,
                retry_after: None,
                body: body.as_bytes().to_vec(),
            });
    }
    /// The `text` of every request sent, in order.
    pub fn texts(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|r| match &r.body {
                vesper_notify::RequestBody::Json(v) => {
                    v["text"].as_str().unwrap_or_default().to_string()
                }
                other => panic!("unexpected body {other:?}"),
            })
            .collect()
    }
    pub fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}
impl NotifyTransport for FakeTelegram {
    fn http<'a>(
        &'a self,
        request: &'a vesper_notify::HttpRequest,
        _network: &'a Network,
    ) -> BoxFuture<'a, Result<vesper_notify::Response, vesper_notify::Verdict>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(request.clone());
            Ok(self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(vesper_notify::Response {
                    status: 200,
                    retry_after: None,
                    body: br#"{"ok":true,"result":{"message_id":1}}"#.to_vec(),
                }))
        })
    }

    fn mail<'a>(
        &'a self,
        _request: &'a vesper_notify::MailRequest,
        _network: &'a Network,
    ) -> BoxFuture<'a, vesper_notify::Verdict> {
        Box::pin(async { panic!("no mail channels in engine tests") })
    }
}

pub fn http(name: &str) -> Check {
    Check {
        name: name.into(),
        url: Some("https://10.0.0.5/".into()),
        ..Check::default()
    }
}

pub fn push(name: &str) -> Check {
    Check {
        kind: CheckKind::Push,
        name: name.into(),
        expected_interval_seconds: 300,
        grace_seconds: 60,
        ..Check::default()
    }
}

pub fn service(name: &str, checks: &[(&str, Check)]) -> Service {
    Service {
        name: name.into(),
        checks: checks
            .iter()
            .map(|(k, c)| (k.to_string(), c.clone()))
            .collect(),
        ..Service::default()
    }
}

pub fn config(machines: &[(&str, Vec<(&str, Service)>)]) -> Config {
    Config {
        telegram: Some(TelegramConfig {
            bot_token: SecretRef::File("bot_token".into()),
            chat_id: SecretRef::File("chat_id".into()),
            dashboard_url: "https://vesper.example".into(),
            reminder_seconds: None,
        }),
        machines: machines
            .iter()
            .map(|(m, services)| {
                (
                    m.to_string(),
                    Machine {
                        name: m.to_string(),
                        services: services
                            .iter()
                            .map(|(s, svc)| (s.to_string(), svc.clone()))
                            .collect(),
                        ..Machine::default()
                    },
                )
            })
            .collect(),
        ..Config::default()
    }
}

pub fn loaded(config: Config, revision: &str) -> LoadedConfig {
    LoadedConfig {
        config,
        revision: revision.into(),
        sources: BTreeMap::new(),
        writable: true,
    }
}

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub clock: Arc<ManualClock>,
    pub prober: Arc<FakeProber>,
    pub telegram: Arc<FakeTelegram>,
    pub engine: Arc<Mutex<Engine>>,
}

pub fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
}

impl Harness {
    pub fn new(config: Config) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bot_token"), "123456:TOKEN-abc\n").unwrap();
        std::fs::write(dir.path().join("chat_id"), "-100987654\n").unwrap();
        let clock = Arc::new(ManualClock(StdMutex::new(start())));
        let prober = Arc::new(FakeProber::default());
        let telegram = Arc::new(FakeTelegram::default());
        let engine = open(dir.path(), &clock, &prober, &telegram, loaded(config, "r1"));
        Harness {
            dir,
            clock,
            prober,
            telegram,
            engine,
        }
    }

    /// Simulates a process restart against the same database.
    pub fn restart(&mut self, config: Config) {
        self.engine = open(
            self.dir.path(),
            &self.clock,
            &self.prober,
            &self.telegram,
            loaded(config, "r1"),
        );
    }

    pub async fn tick(&self) {
        tick(&self.engine).await;
    }

    /// Advances the clock then ticks.
    pub async fn step(&self, seconds: i64) {
        self.clock.advance(seconds);
        self.tick().await;
    }

    pub async fn snapshot(&self) -> Snapshot {
        self.engine.lock().await.snapshot().unwrap()
    }

    /// Delivery status of the only notification channel.
    pub async fn channel_status(&self) -> String {
        let channels = self.snapshot().await.notifications;
        assert_eq!(channels.len(), 1);
        channels[0].status.clone()
    }

    pub async fn service(&self, id: &str) -> ServiceView {
        self.snapshot()
            .await
            .machines
            .into_iter()
            .flat_map(|m| m.services)
            .find(|s| s.id == id)
            .unwrap()
    }

    pub async fn incidents(&self, service_id: &str) -> Vec<IncidentView> {
        self.snapshot()
            .await
            .incidents
            .into_iter()
            .filter(|i| i.service_id == service_id)
            .collect()
    }
}

fn open(
    dir: &Path,
    clock: &Arc<ManualClock>,
    prober: &Arc<FakeProber>,
    telegram: &Arc<FakeTelegram>,
    loaded: LoadedConfig,
) -> Arc<Mutex<Engine>> {
    let db: PathBuf = dir.join("vesper.db");
    let engine = Engine::open_with(
        &db,
        loaded,
        dir.to_path_buf(),
        EngineOptions {
            clock: clock.clone(),
            prober: prober.clone(),
            transport: telegram.clone(),
        },
    )
    .unwrap();
    Arc::new(Mutex::new(engine))
}

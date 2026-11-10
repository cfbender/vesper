mod common;

use common::*;
use std::sync::Arc;
use vesper_core::*;

const WEB: &str = "jelly/web/ui";

fn web_config() -> Config {
    config(&[(
        "jelly",
        vec![("web", service("Web", &[("ui", http("UI"))]))],
    )])
}

/// Runs one 30 s step per character: F = failing, P = passing.
async fn steps(h: &Harness, check_id: &str, pattern: &str) {
    for c in pattern.chars() {
        h.prober.set(check_id, c == 'P');
        h.step(30).await;
    }
}

fn assert_safe(h: &Harness) {
    for text in h.telegram.texts() {
        assert!(
            !text.contains("body-evidence"),
            "probe evidence leaked: {text}"
        );
        assert!(!text.contains("TOKEN"), "credential leaked: {text}");
        assert!(text.len() <= 4096);
    }
}

#[tokio::test]
async fn flapping_check_opens_one_incident_and_needs_two_consecutive_passes() {
    let h = Harness::new(web_config());
    h.prober.set(WEB, false);
    h.tick().await; // t0 F: suspect
    steps(&h, WEB, "FPFF").await; // F F P F F: never three in a row
    let svc = h.service("jelly/web").await;
    assert_eq!(svc.health, Health::Suspect);
    assert_eq!(svc.checks[0].failures, 2);
    assert!(h.incidents("jelly/web").await.is_empty());
    assert_eq!(h.telegram.count(), 0);

    steps(&h, WEB, "F").await; // third consecutive failure
    assert_eq!(h.service("jelly/web").await.health, Health::Down);
    assert_eq!(h.telegram.count(), 1);
    assert!(h.telegram.texts()[0].contains("DOWN Web (jelly/web)"));
    assert!(h.telegram.texts()[0].contains("failing: UI"));

    steps(&h, WEB, "P").await;
    assert_eq!(
        h.service("jelly/web").await.checks[0].health,
        Health::Recovering
    );
    steps(&h, WEB, "FP").await; // failed recovery resets passes, no new incident
    let incidents = h.incidents("jelly/web").await;
    assert_eq!(incidents.len(), 1);
    assert!(incidents[0].resolved_at.is_none());
    assert_eq!(h.service("jelly/web").await.health, Health::Recovering);

    steps(&h, WEB, "P").await; // second consecutive pass recovers
    let incidents = h.incidents("jelly/web").await;
    assert_eq!(incidents.len(), 1);
    assert!(incidents[0].resolved_at.is_some());
    assert!(incidents[0]
        .events
        .iter()
        .any(|e| e.message.contains("recovery progress reset")));
    assert_eq!(h.service("jelly/web").await.health, Health::Healthy);
    steps(&h, WEB, "PPP").await;
    let texts = h.telegram.texts();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts[1].contains("RECOVERED Web (jelly/web)"));
    assert!(texts[1].contains(&format!(
        "https://vesper.example/incidents/{}",
        incidents[0].id
    )));
    assert_safe(&h);
}

#[tokio::test]
async fn check_stats_report_uptime_windows_passing_latency_and_state_changes() {
    let h = Harness::new(web_config());
    h.prober.set(WEB, false);
    h.tick().await; // 12:00:00 F: suspect
    steps(&h, WEB, "FFPP").await; // down at 12:01:00, healthy at 12:02:00
    h.clock.advance(88 * 60); // 13:30:00
    h.tick().await;
    steps(&h, WEB, "P").await;

    let stats = h.engine.lock().await.check_stats(WEB).unwrap();
    let window = |seconds: u64| {
        stats
            .windows
            .iter()
            .find(|w| w.seconds == seconds)
            .unwrap()
            .stats
            .clone()
    };
    let passing = |passes, failures| SampleStats {
        passes,
        failures,
        latency_avg_ms: Some(12.0),
        latency_min_ms: Some(12),
        latency_max_ms: Some(12),
    };
    assert_eq!(
        window(3600),
        passing(2, 0),
        "12:0x samples are over an hour old"
    );
    assert_eq!(
        window(86400),
        passing(4, 3),
        "failed 5000 ms probes are not latency"
    );
    assert_eq!(window(365 * 86400), passing(4, 3));
    assert_eq!(
        stats.hours,
        vec![
            StatsHour {
                hour: "2026-11-10T12:00:00.000Z".into(),
                stats: passing(2, 3)
            },
            StatsHour {
                hour: "2026-11-10T13:00:00.000Z".into(),
                stats: passing(2, 0)
            },
        ]
    );
    let events: Vec<_> = stats
        .events
        .iter()
        .map(|e| (e.at.as_str(), e.health))
        .collect();
    assert_eq!(
        events,
        [
            ("2026-11-10T12:02:00.000Z", Health::Healthy),
            ("2026-11-10T12:01:30.000Z", Health::Recovering),
            ("2026-11-10T12:01:00.000Z", Health::Down),
            ("2026-11-10T12:00:00.000Z", Health::Suspect),
        ],
        "a pass after the gap is no state change"
    );
    assert_eq!(stats.events[2].message, "HTTP 503 body-evidence-xyz");

    let h = Harness::new(backup_config());
    h.engine.lock().await.heartbeat(JOB, true).unwrap();
    h.step(361).await; // missed deadline
    let stats = h.engine.lock().await.check_stats(JOB).unwrap();
    assert_eq!(
        stats.windows[0].stats,
        SampleStats {
            passes: 1,
            ..SampleStats::default()
        },
        "heartbeats have no latency and a missed deadline adds no sample"
    );
    let events: Vec<_> = stats.events.iter().map(|e| e.health).collect();
    assert_eq!(events, [Health::Down, Health::Healthy]);
    assert!(stats.events[0]
        .message
        .starts_with("No heartbeat by deadline"));
}

#[tokio::test]
async fn restart_preserves_incident_and_delivery_and_requires_new_evidence() {
    let cfg = config(&[(
        "jelly",
        vec![
            ("web", service("Web", &[("ui", http("UI"))])),
            ("api", service("API", &[("ping", http("Ping"))])),
        ],
    )]);
    let mut h = Harness::new(cfg.clone());
    let api = "jelly/api/ping";
    h.prober.set(WEB, false);
    h.tick().await;
    h.prober.set(api, false);
    steps(&h, WEB, "FF").await; // web down at t60; api suspect with 2 failures
    assert_eq!(h.service("jelly/api").await.checks[0].failures, 2);
    assert_eq!(h.telegram.count(), 1);

    // A quick restart: the persisted samples are only 10 s old but predate
    // this process, so they are stale evidence.
    h.clock.advance(10);
    h.restart(cfg.clone());
    let snap = h.snapshot().await;
    let web = h.service("jelly/web").await;
    assert_eq!(web.health, Health::Stale, "no current evidence yet");
    assert_eq!(web.checks[0].health, Health::Stale);
    assert_eq!(web.checks[0].history.len(), 3, "the gap adds no samples");
    assert_eq!(h.service("jelly/api").await.checks[0].health, Health::Stale);
    let incident = snap
        .incidents
        .iter()
        .find(|i| i.service_id == "jelly/web")
        .unwrap();
    assert!(
        incident.resolved_at.is_none(),
        "open incident survives restart"
    );
    assert!(incident.notification_state.starts_with("outage sent"));
    assert!(incident
        .events
        .iter()
        .any(|e| e.message.contains("Vesper restarted")));

    // One failure after the restart must not complete api's earlier streak.
    h.tick().await;
    let api_view = h.service("jelly/api").await;
    assert_eq!(api_view.health, Health::Suspect);
    assert_eq!(api_view.checks[0].failures, 1);
    assert!(h.incidents("jelly/api").await.is_empty());
    assert_eq!(h.service("jelly/web").await.health, Health::Down);
    assert_eq!(h.telegram.count(), 1, "no duplicate outage after restart");

    // A pre-restart recovery pass does not count toward recovery either.
    h.prober.set(api, true);
    steps(&h, WEB, "P").await;
    assert_eq!(h.service("jelly/web").await.health, Health::Recovering);
    h.clock.advance(5);
    h.restart(cfg);
    h.tick().await; // first pass of the new period: 1 of 2
    assert!(h.incidents("jelly/web").await[0].resolved_at.is_none());
    assert_eq!(h.incidents("jelly/web").await.len(), 1);
    steps(&h, WEB, "P").await;
    assert!(h.incidents("jelly/web").await[0].resolved_at.is_some());
    assert_eq!(h.telegram.count(), 2);
    assert!(h.telegram.texts()[1].contains("RECOVERED Web"));
}

fn backup_config() -> Config {
    config(&[(
        "nas",
        vec![("backup", service("Backup", &[("job", push("Nightly"))]))],
    )])
}
const JOB: &str = "nas/backup/job";

#[tokio::test]
async fn push_deadline_opens_once_and_one_heartbeat_recovers() {
    let h = Harness::new(backup_config());
    h.engine.lock().await.heartbeat(JOB, true).unwrap();
    h.step(300).await;
    assert_eq!(h.service("nas/backup").await.health, Health::Healthy);
    h.step(59).await; // t359, deadline is t360
    assert!(h.incidents("nas/backup").await.is_empty());
    h.step(2).await; // t361: one missed cadence plus grace
    assert_eq!(h.service("nas/backup").await.health, Health::Down);
    assert_eq!(h.telegram.count(), 1);
    for _ in 0..5 {
        h.step(600).await;
    }
    let svc = h.service("nas/backup").await;
    assert_eq!(
        svc.checks[0].history.len(),
        1,
        "deadline ticks add no failed samples"
    );
    assert_eq!(h.incidents("nas/backup").await.len(), 1);
    assert_eq!(h.telegram.count(), 1);

    h.engine.lock().await.heartbeat(JOB, true).unwrap();
    assert!(h.incidents("nas/backup").await[0].resolved_at.is_some());
    h.tick().await;
    assert_eq!(h.telegram.count(), 2);
    assert!(h.telegram.texts()[1].contains("RECOVERED Backup"));

    // An explicit unhealthy report is a producer-confirmed failure.
    h.engine.lock().await.heartbeat(JOB, false).unwrap();
    assert_eq!(h.incidents("nas/backup").await.len(), 2);
    assert!(h
        .engine
        .lock()
        .await
        .heartbeat("jelly/web/ui", true)
        .is_err());
}

#[tokio::test]
async fn push_restart_gap_is_stale_not_a_missed_deadline() {
    let mut h = Harness::new(backup_config());
    h.engine.lock().await.heartbeat(JOB, true).unwrap();
    h.clock.advance(1000); // Vesper is down across the deadline
    h.restart(backup_config());
    h.tick().await;
    let svc = h.service("nas/backup").await;
    assert_eq!(svc.checks[0].health, Health::Stale);
    assert!(h.incidents("nas/backup").await.is_empty());
    h.step(360).await; // deadline restarts from the restart
    assert!(h.incidents("nas/backup").await.is_empty());
    h.step(1).await;
    assert_eq!(h.incidents("nas/backup").await.len(), 1);
}

#[tokio::test]
async fn recovery_needs_every_enabled_check_fresh_and_healthy() {
    let port = Check {
        kind: CheckKind::Tcp,
        name: "Port".into(),
        host: Some("10.0.0.5".into()),
        port: Some(8096),
        interval_seconds: Some(600),
        stale_after_seconds: Some(90),
        ..Check::default()
    };
    let legacy = Check {
        enabled: false,
        ..http("Legacy")
    };
    let cfg = config(&[(
        "jelly",
        vec![
            (
                "app",
                service(
                    "App",
                    &[("web", http("Web")), ("port", port), ("legacy", legacy)],
                ),
            ),
            ("duo", service("Duo", &[("a", http("A")), ("b", http("B"))])),
        ],
    )]);
    let h = Harness::new(cfg);
    let (web, a, b) = ("jelly/app/web", "jelly/duo/a", "jelly/duo/b");
    h.prober.set("jelly/app/legacy", false);
    h.prober.set(web, false);
    h.prober.set(a, false);
    h.tick().await;
    h.step(30).await;
    h.step(30).await; // t60: web and a are down
    assert_eq!(h.service("jelly/app").await.health, Health::Down);
    assert_eq!(h.service("jelly/duo").await.health, Health::Down);

    h.prober.set(web, true);
    h.prober.set(a, true);
    h.prober.set(b, false);
    h.step(30).await;
    h.step(30).await; // t120: web and a healthy, b suspect, port stale
    let app = h.service("jelly/app").await;
    assert_eq!(
        app.health,
        Health::Stale,
        "open incident without current evidence"
    );
    let port_view = app
        .checks
        .iter()
        .find(|c| c.id == "jelly/app/port")
        .unwrap();
    assert_eq!(port_view.health, Health::Stale);
    assert!(h.incidents("jelly/app").await[0].resolved_at.is_none());
    let duo = h.service("jelly/duo").await;
    assert_eq!(
        duo.checks.iter().find(|c| c.id == b).unwrap().health,
        Health::Suspect
    );
    assert!(h.incidents("jelly/duo").await[0].resolved_at.is_none());

    h.prober.set(b, true);
    h.engine.lock().await.recheck("jelly/app").unwrap();
    h.step(30).await; // port reruns via recheck; b passes
    assert!(h.incidents("jelly/app").await[0].resolved_at.is_some());
    assert!(h.incidents("jelly/duo").await[0].resolved_at.is_some());
    assert_eq!(h.incidents("jelly/duo").await.len(), 1);
    assert_eq!(h.prober.calls("jelly/app/legacy"), 0);
    let app = h.service("jelly/app").await;
    let legacy_view = app
        .checks
        .iter()
        .find(|c| c.id == "jelly/app/legacy")
        .unwrap();
    assert_eq!(legacy_view.health, Health::Paused);
}

#[tokio::test]
async fn silence_defers_outage_until_expiry_when_still_down() {
    let h = Harness::new(web_config());
    let until = start() + chrono::Duration::seconds(600);
    h.engine
        .lock()
        .await
        .silence("jelly/web", Some(until))
        .unwrap();
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FF").await;
    let incident = &h.incidents("jelly/web").await[0];
    assert!(incident
        .notification_state
        .starts_with("suppressed: silenced until"));
    assert!(h.service("jelly/web").await.silenced_until.is_some());
    while h.clock.get() < until - chrono::Duration::seconds(30) {
        h.step(30).await;
    }
    assert_eq!(h.telegram.count(), 0, "silence suppresses notifications");
    assert!(h.prober.calls(WEB) > 15, "silence does not stop execution");
    h.step(30).await;
    assert_eq!(h.telegram.count(), 1);
    assert!(h.telegram.texts()[0].contains("DOWN Web"));
}

#[tokio::test]
async fn wholly_suppressed_resolved_incident_stays_history_only() {
    let h = Harness::new(web_config());
    let until = start() + chrono::Duration::seconds(3600);
    h.engine
        .lock()
        .await
        .silence("jelly/web", Some(until))
        .unwrap();
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FFPP").await;
    assert!(h.incidents("jelly/web").await[0].resolved_at.is_some());
    h.step(3600).await;
    h.step(30).await;
    assert_eq!(h.telegram.count(), 0);
    let first = &h.incidents("jelly/web").await[0];
    assert_eq!(
        first.notification_state,
        "history only: the outage was never delivered"
    );

    // A later, unsuppressed outage is delivered normally.
    steps(&h, WEB, "FFF").await;
    assert_eq!(h.telegram.count(), 1);
    assert!(!h.telegram.texts()[0].contains(&format!("/incidents/{}\n", first.id)));
}

#[tokio::test]
async fn recovery_after_delivered_outage_waits_for_silence_to_expire() {
    let h = Harness::new(web_config());
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FF").await;
    assert_eq!(h.telegram.count(), 1);
    let until = h.clock.get() + chrono::Duration::seconds(1000);
    h.engine
        .lock()
        .await
        .silence("jelly/web", Some(until))
        .unwrap();
    steps(&h, WEB, "PP").await;
    let incident = &h.incidents("jelly/web").await[0];
    assert!(incident.resolved_at.is_some());
    assert!(incident
        .notification_state
        .starts_with("recovery deferred: silenced"));
    assert_eq!(h.telegram.count(), 1);
    h.step(1000).await;
    assert_eq!(h.telegram.count(), 2);
    assert!(h.telegram.texts()[1].contains("RECOVERED Web"));
}

#[tokio::test]
async fn weekly_maintenance_suppresses_and_disabled_notifications_stay_quiet() {
    let mut cfg = config(&[(
        "jelly",
        vec![
            ("web", service("Web", &[("ui", http("UI"))])),
            ("lab", service("Lab", &[("ui", http("UI"))])),
        ],
    )]);
    let week = chrono::Duration::weeks(1);
    let jelly = cfg.machines.get_mut("jelly").unwrap();
    jelly.services.get_mut("web").unwrap().maintenance = vec![MaintenanceWindow {
        start: (start() - week - chrono::Duration::minutes(10)).to_rfc3339(),
        end: (start() - week + chrono::Duration::minutes(20)).to_rfc3339(),
        repeat_weekly: true,
    }];
    jelly.services.get_mut("lab").unwrap().notifications = false;
    let h = Harness::new(cfg);
    h.prober.set(WEB, false);
    h.prober.set("jelly/lab/ui", false);
    h.tick().await;
    steps(&h, WEB, "FF").await;
    assert!(h.service("jelly/web").await.maintenance);
    assert_eq!(h.incidents("jelly/lab").await.len(), 1);
    while h.clock.get() < start() + chrono::Duration::minutes(20) - chrono::Duration::seconds(30) {
        h.step(30).await;
    }
    assert_eq!(h.telegram.count(), 0);
    h.step(30).await; // window ends at +20 min
    assert!(!h.service("jelly/web").await.maintenance);
    assert_eq!(h.telegram.count(), 1);
    assert!(h.telegram.texts()[0].contains("jelly/web"));
    assert!(!h.telegram.texts()[0].contains("jelly/lab"));
    assert_eq!(
        h.incidents("jelly/lab").await[0].notification_state,
        "notifications disabled for this service"
    );
}

#[tokio::test]
async fn retry_after_is_durable_and_the_retry_sends_current_state() {
    let quick = Check {
        failure_threshold: Some(1),
        recovery_threshold: Some(1),
        ..http("Y")
    };
    let cfg = config(&[(
        "jelly",
        vec![
            ("a", service("Alpha", &[("x", http("X"))])),
            ("b", service("Beta", &[("y", quick)])),
        ],
    )]);
    let mut h = Harness::new(cfg.clone());
    h.telegram.reply(
        429,
        r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 120","parameters":{"retry_after":120}}"#,
    );
    h.prober.set("jelly/a/x", false);
    h.tick().await;
    steps(&h, "jelly/a/x", "FF").await; // t60: Alpha down, attempt rate limited
    assert_eq!(h.telegram.count(), 1);
    assert!(h
        .channel_status()
        .await
        .starts_with("rate limited by Telegram"));

    h.prober.set("jelly/b/y", false);
    h.step(30).await; // t90: Beta down
    h.prober.set("jelly/b/y", true);
    h.step(30).await; // t120: Beta recovered
    assert_eq!(h.incidents("jelly/b").await.len(), 1);
    assert_eq!(h.telegram.count(), 1, "no attempts inside retry_after");

    h.restart(cfg);
    h.step(30).await; // t150, still inside retry_after after restart
    assert_eq!(h.telegram.count(), 1, "retry_after survives restart");
    h.step(30).await; // t180: retry_after elapsed
    assert_eq!(h.telegram.count(), 2);
    let summary = &h.telegram.texts()[1];
    assert!(summary.contains("DOWN Alpha (jelly/a)"));
    assert!(
        !summary.contains("Beta"),
        "obsolete outage+recovery is not replayed: {summary}"
    );
    assert_eq!(
        h.incidents("jelly/b").await[0].notification_state,
        "history only: the outage was never delivered"
    );
    assert!(h.channel_status().await.starts_with("ok;"));
}

#[tokio::test]
async fn telegram_rejection_is_exposed_and_stops_until_test_or_config_change() {
    let h = Harness::new(web_config());
    let unauthorized = r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#;
    h.telegram.reply(401, unauthorized);
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FF").await;
    assert_eq!(h.telegram.count(), 1);
    let status = h.channel_status().await;
    assert!(
        status.starts_with("error: Telegram API 401: Unauthorized"),
        "{status}"
    );
    for _ in 0..5 {
        h.step(300).await;
    }
    assert_eq!(h.telegram.count(), 1, "rejections are not retried blindly");

    let message = vesper_engine::test_channel(h.engine.clone(), "telegram")
        .await
        .unwrap();
    assert!(message.contains("not a read receipt"));
    h.tick().await;
    assert_eq!(h.telegram.count(), 3);
    assert!(h.telegram.texts()[2].contains("DOWN Web"));

    h.telegram.reply(
        403,
        r#"{"ok":false,"error_code":403,"description":"Forbidden: bot was blocked by the user"}"#,
    );
    steps(&h, WEB, "PP").await;
    assert_eq!(h.telegram.count(), 4);
    steps(&h, WEB, "PPPP").await;
    assert_eq!(h.telegram.count(), 4);
    assert!(h.channel_status().await.contains("bot was blocked"));
    h.engine
        .lock()
        .await
        .replace_config(loaded(web_config(), "r2"))
        .unwrap();
    h.tick().await;
    assert_eq!(h.telegram.count(), 5);
    assert!(h.telegram.texts()[4].contains("RECOVERED Web"));

    h.telegram.reply(
        429,
        r#"{"ok":false,"error_code":429,"parameters":{"retry_after":5}}"#,
    );
    let err = vesper_engine::test_channel(h.engine.clone(), "telegram")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("retry after 5 seconds"));
    assert_safe(&h);
}

#[tokio::test]
async fn dependency_failure_groups_notifications_and_rechecks_children_on_recovery() {
    let child = Check {
        failure_threshold: Some(1),
        recovery_threshold: Some(1),
        interval_seconds: Some(600),
        stale_after_seconds: Some(1200),
        ..http("Web")
    };
    let mut cfg = config(&[(
        "jelly",
        vec![
            ("host", service("Host", &[("ping", http("Ping"))])),
            (
                "media",
                Service {
                    depends_on: vec!["jelly/host".into()],
                    ..service("Media", &[("web", child.clone())])
                },
            ),
            (
                "other",
                Service {
                    depends_on: vec!["jelly/host".into()],
                    ..service("Other", &[("web", child.clone())])
                },
            ),
        ],
    )]);
    cfg.machines.insert(
        "nas".into(),
        Machine {
            name: "NAS".into(),
            services: [(
                "app".to_string(),
                Service {
                    depends_on: vec!["jelly/host".into()],
                    ..service("App", &[("web", child)])
                },
            )]
            .into(),
            ..Machine::default()
        },
    );
    cfg.machines.get_mut("jelly").unwrap().host_service = Some("host".into());
    let h = Harness::new(cfg);
    for id in [
        "jelly/host/ping",
        "jelly/media/web",
        "jelly/other/web",
        "nas/app/web",
    ] {
        h.prober.set(id, false);
    }
    h.tick().await; // children confirmed down at once; host only suspect
    assert_eq!(h.incidents("nas/app").await.len(), 1);
    assert_eq!(
        h.incidents("jelly/media").await[0].notification_state,
        "waiting to group with dependency jelly/host"
    );
    assert_eq!(h.telegram.count(), 0);
    h.step(30).await;
    h.step(30).await; // host confirmed down
    assert_eq!(h.telegram.count(), 1, "{:?}", h.telegram.texts());
    let text = &h.telegram.texts()[0];
    assert!(text.contains("DOWN Host (jelly/host)"), "{text}");
    assert!(
        text.contains("Also down, depending on jelly/host"),
        "{text}"
    );
    for child in [
        "Media (jelly/media)",
        "Other (jelly/other)",
        "App (nas/app)",
    ] {
        assert!(text.contains(child), "{text}");
        assert!(!text.contains(&format!("DOWN {child}")), "{text}");
    }
    let media = &h.incidents("jelly/media").await[0];
    assert_eq!(media.correlated_with.as_deref(), Some("jelly/host"));
    assert!(media
        .events
        .iter()
        .any(|e| e.message.contains("not proven root cause")));
    assert_eq!(
        h.prober.calls("jelly/media/web"),
        1,
        "children keep their own schedule"
    );

    for id in [
        "jelly/host/ping",
        "jelly/media/web",
        "jelly/other/web",
        "nas/app/web",
    ] {
        h.prober.set(id, true);
    }
    h.step(30).await;
    h.step(30).await; // host recovered at t120; children were last run at t0
    assert_eq!(h.telegram.count(), 2);
    assert!(h.telegram.texts()[1].contains("RECOVERED Host"));
    h.step(1).await; // post-recovery recheck runs children before their interval
    assert_eq!(h.prober.calls("nas/app/web"), 2);
    assert!(h.incidents("nas/app").await[0].resolved_at.is_some());
    assert_eq!(h.telegram.count(), 3);
    let recovered = &h.telegram.texts()[2];
    assert!(recovered.contains("RECOVERED Media") && recovered.contains("RECOVERED App"));
    assert!(recovered.ends_with("https://vesper.example/incidents"));
}

#[tokio::test]
async fn paused_service_does_not_run_and_is_not_healthy() {
    let mut cfg = config(&[(
        "nas",
        vec![(
            "backup",
            service("Backup", &[("job", push("Nightly")), ("ui", http("UI"))]),
        )],
    )]);
    cfg.machines
        .get_mut("nas")
        .unwrap()
        .services
        .get_mut("backup")
        .unwrap()
        .paused = true;
    let h = Harness::new(cfg.clone());
    h.prober.set("nas/backup/ui", false);
    for _ in 0..10 {
        h.step(300).await;
    }
    let svc = h.service("nas/backup").await;
    assert_eq!(svc.health, Health::Paused);
    assert!(svc.checks.iter().all(|c| c.health == Health::Paused));
    assert_eq!(h.prober.calls("nas/backup/ui"), 0);
    assert!(h.incidents("nas/backup").await.is_empty());
    assert!(h.engine.lock().await.recheck("nas/backup").is_err());
    h.engine.lock().await.heartbeat(JOB, true).unwrap();
    assert_eq!(h.service("nas/backup").await.checks[0].history.len(), 0);

    let unpaused = backup_config();
    h.engine
        .lock()
        .await
        .replace_config(loaded(unpaused, "r2"))
        .unwrap();
    h.step(360).await; // push deadline counts from resume, not from start
    assert!(h.incidents("nas/backup").await.is_empty());
    h.step(1).await;
    assert_eq!(h.incidents("nas/backup").await.len(), 1);
}

#[tokio::test]
async fn runs_never_overlap_and_concurrency_is_bounded() {
    let mut cfg = config(&[(
        "jelly",
        vec![(
            "web",
            service(
                "Web",
                &[("a", http("A")), ("b", http("B")), ("c", http("C"))],
            ),
        )],
    )]);
    cfg.network.max_concurrent = 2;
    let h = Harness::new(cfg);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *h.prober.gate.lock().unwrap() = Some(gate.clone());
    let engine = h.engine.clone();
    let first = tokio::spawn(async move { vesper_engine::tick(&engine).await });
    while *h.prober.running.lock().unwrap() < 2 {
        tokio::task::yield_now().await;
    }
    h.step(100).await; // everything is overdue, but two runs are in flight
    let total = |h: &Harness| {
        ["a", "b", "c"]
            .iter()
            .map(|c| h.prober.calls(&format!("jelly/web/{c}")))
            .sum::<usize>()
    };
    assert_eq!(total(&h), 2);
    gate.add_permits(100);
    first.await.unwrap();
    h.tick().await; // the longest-waiting check goes first
    assert_eq!(total(&h), 4);
    assert_eq!(h.prober.calls("jelly/web/c"), 1);
    assert_eq!(*h.prober.max_running.lock().unwrap(), 2);
}

#[tokio::test]
async fn acknowledgement_is_awareness_and_stops_opt_in_reminders() {
    let mut cfg = web_config();
    cfg.telegram.as_mut().unwrap().reminder_seconds = Some(600);
    let h = Harness::new(cfg);
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FF").await; // t60 outage
    for _ in 0..20 {
        h.step(30).await; // t660
    }
    assert_eq!(h.telegram.count(), 2);
    assert!(h.telegram.texts()[1].contains("STILL DOWN Web"));
    let id = h.incidents("jelly/web").await[0].id;
    h.engine.lock().await.acknowledge(id, true).unwrap();
    assert_eq!(h.service("jelly/web").await.health, Health::Down);
    let incident = &h.incidents("jelly/web").await[0];
    assert!(incident.acknowledged_at.is_some());
    for _ in 0..40 {
        h.step(30).await;
    }
    assert_eq!(h.telegram.count(), 2);
    steps(&h, WEB, "PP").await;
    assert_eq!(h.telegram.count(), 3);
    assert!(h.engine.lock().await.acknowledge(9999, true).is_err());
}

#[tokio::test]
async fn removed_service_closes_its_incident_and_audit_is_recorded() {
    let h = Harness::new(web_config());
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FF").await;
    {
        let mut e = h.engine.lock().await;
        e.audit("config.reload", "machines/jelly").unwrap();
        e.replace_config(loaded(config(&[]), "r2")).unwrap();
        e.set_config_error(Some("machines/x.yaml: invalid".into()));
    }
    h.tick().await;
    let snap = h.snapshot().await;
    assert_eq!(snap.config_revision, "r2");
    assert_eq!(
        snap.config_error.as_deref(),
        Some("machines/x.yaml: invalid")
    );
    assert!(snap.incidents[0].resolved_at.is_some());
    assert_eq!(h.telegram.count(), 2);
    assert!(h.telegram.texts()[1].contains("CLOSED (removed from configuration)"));
}

#[tokio::test]
async fn late_dependent_is_held_behind_a_delivered_dependency_outage() {
    let deps = |name: &str, check: Check| Service {
        depends_on: vec!["jelly/host".into()],
        ..service(name, &[("web", check)])
    };
    let quick = Check {
        recovery_threshold: Some(1),
        ..http("Web")
    };
    let mut cfg = config(&[(
        "jelly",
        vec![
            ("host", service("Host", &[("ping", http("Ping"))])),
            ("app", deps("App", http("Web"))),
            ("cache", deps("Cache", quick)),
            // Same machine, no declared dependency: host_service does not group it.
            ("solo", service("Solo", &[("web", http("Web"))])),
        ],
    )]);
    cfg.machines.get_mut("jelly").unwrap().host_service = Some("host".into());
    let h = Harness::new(cfg);
    let (host, app, cache, solo) = (
        "jelly/host/ping",
        "jelly/app/web",
        "jelly/cache/web",
        "jelly/solo/web",
    );
    h.prober.set(host, false);
    h.tick().await;
    h.step(30).await;
    h.step(30).await; // t60: host down and notified
    assert_eq!(h.telegram.count(), 1);
    for id in [app, cache, solo] {
        h.prober.set(id, false);
    }
    for _ in 0..3 {
        h.step(30).await; // t150: app, cache and solo confirmed down
    }
    assert_eq!(h.telegram.count(), 2, "{:?}", h.telegram.texts());
    let text = &h.telegram.texts()[1];
    assert!(text.contains("DOWN Solo (jelly/solo)"), "{text}");
    assert!(!text.contains("App") && !text.contains("Cache"), "{text}");
    let held = &h.incidents("jelly/app").await[0];
    assert_eq!(held.correlated_with.as_deref(), Some("jelly/host"));
    assert!(held
        .notification_state
        .starts_with("held: dependency jelly/host"));
    assert!(h.incidents("jelly/solo").await[0].correlated_with.is_none());
    for _ in 0..5 {
        h.step(30).await;
    }
    assert_eq!(
        h.telegram.count(),
        2,
        "dependents stay held while host is down"
    );
    assert!(h.prober.calls(app) >= 8, "dependent checks keep running");

    h.prober.set(host, true);
    h.prober.set(cache, true);
    h.prober.set(app, false);
    h.step(30).await;
    h.step(30).await; // host recovered; same-instant dependent samples do not count
    assert_eq!(h.telegram.count(), 3);
    assert!(h.telegram.texts()[2].contains("RECOVERED Host"));
    h.step(1).await; // post-recovery recheck
    assert_eq!(h.telegram.count(), 4);
    let text = &h.telegram.texts()[3];
    assert!(text.contains("DOWN App (jelly/app)"), "{text}");
    assert!(!text.contains("Cache"), "{text}");
    assert_eq!(
        h.incidents("jelly/cache").await[0].notification_state,
        "history only: the outage was never delivered"
    );
}

#[tokio::test]
async fn removed_and_readded_check_never_overlaps_or_takes_the_old_result() {
    let h = Harness::new(web_config());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *h.prober.gate.lock().unwrap() = Some(gate.clone());
    h.prober.set(WEB, false);
    let engine = h.engine.clone();
    let first = tokio::spawn(async move { vesper_engine::tick(&engine).await });
    while *h.prober.running.lock().unwrap() < 1 {
        tokio::task::yield_now().await;
    }
    {
        let mut e = h.engine.lock().await;
        e.replace_config(loaded(config(&[]), "r2")).unwrap();
        e.replace_config(loaded(web_config(), "r3")).unwrap();
    }
    h.step(100).await;
    assert_eq!(
        h.prober.calls(WEB),
        1,
        "re-added check waits for the old run"
    );
    gate.add_permits(100);
    first.await.unwrap();
    let ui = &h.service("jelly/web").await.checks[0];
    assert_eq!(ui.health, Health::Unknown, "old run's result is discarded");
    assert!(ui.history.is_empty());
    assert_eq!(ui.failures, 0);
    h.prober.set(WEB, true);
    h.tick().await;
    assert_eq!(h.prober.calls(WEB), 2);
    assert_eq!(
        h.service("jelly/web").await.checks[0].health,
        Health::Healthy
    );
}

#[tokio::test]
async fn pre_restart_heartbeat_cannot_complete_a_recovery() {
    let cfg = config(&[(
        "jelly",
        vec![(
            "web",
            service("Web", &[("ui", http("UI")), ("beat", push("Beat"))]),
        )],
    )]);
    let mut h = Harness::new(cfg.clone());
    h.engine
        .lock()
        .await
        .heartbeat("jelly/web/beat", true)
        .unwrap();
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FF").await; // t60: down because of ui
    h.clock.advance(10);
    h.restart(cfg); // the heartbeat is 70 s old: age-fresh, but pre-restart
    h.prober.set(WEB, true);
    h.tick().await;
    steps(&h, WEB, "P").await; // ui fully recovered with current evidence
    let svc = h.service("jelly/web").await;
    let beat = svc
        .checks
        .iter()
        .find(|c| c.id == "jelly/web/beat")
        .unwrap();
    assert_eq!(beat.health, Health::Stale);
    assert!(h.incidents("jelly/web").await[0].resolved_at.is_none());
    h.engine
        .lock()
        .await
        .heartbeat("jelly/web/beat", true)
        .unwrap();
    assert!(h.incidents("jelly/web").await[0].resolved_at.is_some());
}

#[tokio::test]
async fn oversized_batch_is_split_and_only_named_incidents_count_as_delivered() {
    let long = |prefix: &str, i: usize| format!("{prefix}{i:02}-{}", "x".repeat(90));
    let services: Vec<(String, Service)> = (0..25)
        .map(|i| {
            let checks: Vec<(String, Check)> = (0..5)
                .map(|c| {
                    let check = Check {
                        failure_threshold: Some(1),
                        ..http(&long("check", c))
                    };
                    (format!("c{c}"), check)
                })
                .collect();
            let checks: Vec<(&str, Check)> = checks
                .iter()
                .map(|(k, c)| (k.as_str(), c.clone()))
                .collect();
            (format!("svc{i:02}"), service(&long("Service", i), &checks))
        })
        .collect();
    let mut cfg = config(&[(
        "jelly",
        services
            .iter()
            .map(|(k, s)| (k.as_str(), s.clone()))
            .collect(),
    )]);
    cfg.network.max_concurrent = 200; // all 125 checks run in one pass
    let h = Harness::new(cfg);
    for i in 0..25 {
        for c in 0..5 {
            h.prober.set(&format!("jelly/svc{i:02}/c{c}"), false);
        }
    }
    h.tick().await; // all 25 confirmed down at once
    assert_eq!(h.snapshot().await.incidents.len(), 25);
    for _ in 0..10 {
        h.tick().await; // deliveries continue without advancing time
    }
    let texts = h.telegram.texts();
    assert!(
        texts.len() > 1,
        "a 25-item batch with long names must split"
    );
    for text in &texts {
        assert!(
            text.chars().count() <= 3800,
            "{} chars",
            text.chars().count()
        );
        assert!(text.contains("failing: check00-") && text.contains(" and 2 more"));
    }
    for i in 0..25 {
        let sid = format!("(jelly/svc{i:02})");
        let mentions = texts.iter().filter(|t| t.contains(&sid)).count();
        assert_eq!(mentions, 1, "{sid} must be named in exactly one message");
        let incident = &h.incidents(&format!("jelly/svc{i:02}")).await[0];
        assert!(incident.notification_state.starts_with("outage sent"));
    }
    assert_safe(&h);
}

#[tokio::test]
async fn delayed_summary_describes_stale_evidence_after_restart_honestly() {
    let mut h = Harness::new(backup_config());
    h.engine.lock().await.heartbeat(JOB, true).unwrap();
    h.telegram.reply(
        429,
        r#"{"ok":false,"error_code":429,"parameters":{"retry_after":120}}"#,
    );
    h.step(361).await; // missed deadline; delivery rate limited until t481
    assert_eq!(h.telegram.count(), 1);
    h.clock.advance(9);
    h.restart(backup_config()); // the Down state is now pre-restart evidence
    h.step(120).await; // t490: retry while still without current evidence
    assert_eq!(h.telegram.count(), 2);
    let text = &h.telegram.texts()[1];
    assert!(
        text.contains("OPEN (no current evidence) Backup (nas/backup)"),
        "{text}"
    );
    assert!(!text.contains("DOWN"), "{text}");
}

#[tokio::test]
async fn delayed_summary_reports_recovering_instead_of_down() {
    let h = Harness::new(web_config());
    h.telegram.reply(
        429,
        r#"{"ok":false,"error_code":429,"parameters":{"retry_after":40}}"#,
    );
    h.prober.set(WEB, false);
    h.tick().await;
    steps(&h, WEB, "FFP").await; // t60 down (rate limited until t100), t90 first pass
    h.step(10).await; // t100
    assert_eq!(h.telegram.count(), 2);
    let text = &h.telegram.texts()[1];
    assert!(text.contains("RECOVERING Web (jelly/web)"), "{text}");
    assert!(!text.contains("DOWN"), "{text}");
}

#[tokio::test]
async fn delayed_dependent_outage_does_not_claim_a_recovered_dependency_is_down() {
    let cfg = config(&[(
        "jelly",
        vec![
            ("host", service("Host", &[("ping", http("Ping"))])),
            (
                "app",
                Service {
                    depends_on: vec!["jelly/host".into()],
                    ..service("App", &[("web", http("Web"))])
                },
            ),
        ],
    )]);
    let h = Harness::new(cfg);
    let until = start() + chrono::Duration::seconds(200);
    h.engine
        .lock()
        .await
        .silence("jelly/app", Some(until))
        .unwrap();
    h.prober.set("jelly/host/ping", false);
    h.prober.set("jelly/app/web", false);
    h.tick().await;
    h.step(30).await;
    h.step(30).await; // t60: both down; only Host is notified
    assert_eq!(h.telegram.count(), 1);
    assert!(!h.telegram.texts()[0].contains("App"));
    assert_eq!(
        h.incidents("jelly/app").await[0].correlated_with.as_deref(),
        Some("jelly/host")
    );
    h.prober.set("jelly/host/ping", true);
    for _ in 0..5 {
        h.step(30).await; // host recovered at t120; silence ends at t200
    }
    let texts = h.telegram.texts();
    assert_eq!(texts.len(), 3, "{texts:?}");
    let app = &texts[2];
    assert!(app.contains("DOWN App (jelly/app)"), "{app}");
    assert!(
        app.contains("Depends on jelly/host, which has since recovered"),
        "{app}"
    );
    assert!(!app.contains("which is down"), "{app}");
}

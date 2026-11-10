//! Deterministic randomized edits across file layouts: every save either
//! produces exactly the requested configuration or is refused as
//! export-only with the files untouched, and comments survive unless the
//! value they sat in was removed.

use std::fs;
use std::path::Path;
use vesper_core::config::{self, SaveError};
use vesper_core::{Check, CheckKind, MaintenanceWindow, SecretRef, Service};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const TEXTS: &[&str] = &[
    "Plain",
    "",
    "yes",
    "123",
    "a: b",
    "x # not a comment",
    "Ünïcödé ✓",
    "quote \" and ' marks",
    "multi\nline",
    "  padded  ",
    "{flow}",
    "- dash",
    "null",
    "~",
    "tab\there",
    "https://x.lan/a?b=c#d",
];

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn layouts() -> Vec<Vec<(&'static str, &'static str)>> {
    vec![
        vec![(
            "_index.yaml",
            "# root\nmachines:\n  m:\n    name: M  # machine\n    services:\n      # the host\n      host:\n        name: Host\n        checks:\n          ssh: {kind: tcp, host: 10.0.0.1, port: 22}  # flow\n      app:\n        name: 'App'\n        description: |\n          Literal\n          block\n        depends_on:\n          - m/host\n        checks:\n          web:\n            kind: http\n            url: \"http://10.0.0.1/\"   # quoted\n            headers:\n              X-Key: {env: APP_KEY}\n# trailing\n",
        )],
        vec![
            ("machines/m/_index.yaml", "name: M\n"),
            ("machines/m/services/host.yaml", "# host file\nname: Host\nchecks:\n  ssh:\n    kind: tcp\n    host: 10.0.0.1\n    port: 22\n"),
            ("machines/m/services/app/_index.yaml", "name: App # app\ndepends_on: [m/host]\n"),
            ("machines/m/services/app/checks/web.yaml", "# web\nkind: http\nurl: http://10.0.0.1/\n"),
            ("machines/m/services/app/checks/_index.yaml", "# inline checks\n"),
        ],
        vec![("_index.yaml", "{\"machines\": {\"m\": {\"name\": \"M\", \"services\": {\"host\": {\"name\": \"Host\", \"checks\": {\"ssh\": {\"kind\": \"tcp\", \"host\": \"10.0.0.1\", \"port\": 22}}}, \"app\": {\"name\": \"App\", \"depends_on\": [\"m/host\"], \"checks\": {\"web\": {\"kind\": \"http\", \"url\": \"http://10.0.0.1/\", \"headers\": {\"X-Key\": {\"env\": \"APP_KEY\"}}}}}}}}}\n")],
        vec![("_index.yaml", "{\n  \"machines\": {\n    \"m\": {\n      \"name\": \"M\",\n      \"services\": {\n        \"host\": {\n          \"name\": \"Host\",\n          \"checks\": {\n            \"ssh\": {\n              \"kind\": \"tcp\",\n              \"host\": \"10.0.0.1\",\n              \"port\": 22\n            }\n          }\n        },\n        \"app\": {\n          \"name\": \"App\",\n          \"depends_on\": [\n            \"m/host\"\n          ],\n          \"checks\": {\n            \"web\": {\n              \"kind\": \"http\",\n              \"url\": \"http://10.0.0.1/\",\n              \"headers\": {\n                \"X-Key\": {\n                  \"env\": \"APP_KEY\"\n                }\n              }\n            }\n          }\n        }\n      }\n    }\n  }\n}\n")],
        vec![
            ("machines.yaml", "m:\n  name: M\n  services:\n    host: {name: Host, checks: {ssh: {kind: tcp, host: 10.0.0.1, port: 22}}}\n    app: {name: App}\n"),
        ],
        vec![
            ("machines/m/_index.yaml", "name: M\n"),
            ("machines/m/services/host.yaml", "# just comments\n"),
            ("machines/m/services/app.yaml", ""),
        ],
    ]
}

fn mutate(rng: &mut Rng, svc: &mut Service) {
    for _ in 0..1 + rng.below(3) {
        match rng.below(10) {
            0 => svc.name = rng.pick(TEXTS).to_string().replace(['\n', '\t'], " "),
            1 => svc.description = rng.pick(TEXTS).to_string(),
            2 => svc.paused = !svc.paused,
            3 => svc.notifications = !svc.notifications,
            4 => {
                svc.depends_on = if svc.depends_on.is_empty() {
                    vec!["m/host".into()]
                } else {
                    vec![]
                };
            }
            5 => {
                if rng.chance(50) || svc.maintenance.is_empty() {
                    svc.maintenance.push(MaintenanceWindow {
                        start: "2026-01-01T00:00:00Z".into(),
                        end: format!("2026-01-01T0{}:00:00Z", 1 + rng.below(8)),
                        repeat_weekly: rng.chance(50),
                    });
                } else {
                    svc.maintenance.pop();
                }
            }
            6 => {
                let id = format!("c{}", rng.below(4));
                let check = match rng.below(3) {
                    0 => Check {
                        url: Some(format!("http://10.0.0.{}/", rng.below(9))),
                        body_contains: rng
                            .chance(50)
                            .then(|| rng.pick(TEXTS).to_string())
                            .filter(|s| !s.is_empty()),
                        ..Check::default()
                    },
                    1 => Check {
                        kind: CheckKind::Tcp,
                        host: Some("host.lan".into()),
                        port: Some(1 + rng.below(60000) as u16),
                        ..Check::default()
                    },
                    _ => Check {
                        kind: CheckKind::Push,
                        token: Some(SecretRef::File("beat".into())),
                        ..Check::default()
                    },
                };
                svc.checks.insert(id, check);
            }
            7 => {
                if let Some(k) = svc.checks.keys().next().cloned() {
                    svc.checks.remove(&k);
                }
            }
            8 => {
                if let Some(c) = svc.checks.values_mut().find(|c| c.kind == CheckKind::Http) {
                    if c.headers.is_empty() {
                        c.headers
                            .insert("X-Other".into(), SecretRef::Env("OTHER".into()));
                    } else {
                        c.headers.clear();
                    }
                }
            }
            _ => {
                if let Some(c) = svc.checks.values_mut().next() {
                    c.interval_seconds = if c.interval_seconds.is_some() {
                        None
                    } else {
                        Some(60)
                    };
                    c.name = rng.pick(TEXTS).to_string().replace(['\n', '\t'], " ");
                }
            }
        }
    }
}

fn comments(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            l.find(" #")
                .or(l.starts_with('#').then_some(0))
                .map(|i| l[i..].trim().to_string())
        })
        .collect()
}

fn all_files(root: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().display().to_string(),
                    fs::read_to_string(&p).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn randomized_saves_are_exact_or_refused() {
    for seed in 1..4u64 {
        let mut rng = Rng(0x5eed_1234_abcd_0001 ^ seed.wrapping_mul(0x9E3779B97F4A7C15));
        let (mut saved, mut refused) = (0, 0);
        for (li, layout) in layouts().into_iter().enumerate() {
            let d = tempfile::tempdir().unwrap();
            for (p, c) in &layout {
                write(d.path(), p, c);
            }
            // The seeded comments; generated values may contain `#` text.
            let genuine: Vec<String> = layout.iter().flat_map(|(_, c)| comments(c)).collect();
            for step in 0..60 {
                let l = config::load(d.path())
                    .unwrap_or_else(|e| panic!("layout {li} step {step}: {e:#}"));
                let ids: Vec<String> = l.config.machines["m"].services.keys().cloned().collect();
                let sid = if rng.chance(10) {
                    format!("new{step}")
                } else {
                    rng.pick(&ids).clone()
                };
                let mut svc = l.config.machines["m"]
                    .services
                    .get(&sid)
                    .cloned()
                    .unwrap_or_default();
                mutate(&mut rng, &mut svc);
                if svc.depends_on.contains(&format!("m/{sid}")) {
                    svc.depends_on.clear();
                }
                let before = all_files(d.path());
                let before_comments: Vec<String> = before
                    .iter()
                    .flat_map(|(_, t)| comments(t))
                    .filter(|c| genuine.contains(c))
                    .collect();
                match config::save_service(d.path(), &l.revision, "m", &sid, &svc) {
                    Ok(after) => {
                        saved += 1;
                        let mut expected = l.config.clone();
                        expected
                            .machines
                            .get_mut("m")
                            .unwrap()
                            .services
                            .insert(sid.clone(), svc.clone());
                        assert_eq!(after.config, expected, "layout {li} step {step}");
                        assert_eq!(config::load(d.path()).unwrap().config, expected);
                        // Comments in files that still exist remain, except
                        // those inside removed checks.
                        let now = all_files(d.path());
                        let now_text: String = now.iter().map(|(_, t)| t.as_str()).collect();
                        let removed_check =
                            l.config.machines["m"]
                                .services
                                .get(&sid)
                                .is_some_and(|old| {
                                    old.checks.keys().any(|k| !svc.checks.contains_key(k))
                                });
                        if !removed_check {
                            for c in &before_comments {
                                assert!(
                                    now_text.contains(c.as_str()),
                                    "layout {li} step {step}: lost {c:?}\n{now:#?}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let kind = e.downcast_ref::<SaveError>().cloned();
                        assert!(
                            matches!(kind, Some(SaveError::ExportOnly { .. })),
                            "layout {li} step {step}: {e:#}"
                        );
                        assert_eq!(all_files(d.path()), before, "refused save wrote files");
                        refused += 1;
                    }
                }
            }
        }
        assert!(
            saved > refused * 3,
            "seed {seed}: saved {saved}, refused {refused}"
        );
    }
}

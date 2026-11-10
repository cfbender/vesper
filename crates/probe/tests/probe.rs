//! Probes against real sockets. LAN tests bind a non-loopback local address
//! and allow exactly that /32; override discovery with VESPER_TEST_LAN_IPV4.
//! IPv6 LAN tests run when VESPER_TEST_LAN_IPV6 names a local global/ULA address.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use vesper_core::{Check, CheckDefaults, CheckKind, NetworkPolicy, ProbeResult, SecretRef};

fn lan_ipv4() -> IpAddr {
    let ip = match std::env::var("VESPER_TEST_LAN_IPV4") {
        Ok(raw) => raw.parse().expect("VESPER_TEST_LAN_IPV4 is not an IP"),
        Err(_) => {
            // Routing lookup only; no packet is sent.
            let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
            socket.connect("192.0.2.1:9").unwrap();
            socket.local_addr().unwrap().ip()
        }
    };
    let IpAddr::V4(v4) = ip else {
        panic!("not IPv4")
    };
    assert!(
        !(v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()),
        "no usable non-loopback, non-link-local IPv4 ({ip}); set VESPER_TEST_LAN_IPV4 \
         to a local address such as one added with `ip addr add 10.203.44.1/24 dev dummy0`"
    );
    ip
}

fn host_cidr(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(_) => format!("{ip}/32"),
        IpAddr::V6(_) => format!("{ip}/128"),
    }
}

fn network(cidrs: &[&str]) -> NetworkPolicy {
    NetworkPolicy {
        allowed_cidrs: cidrs.iter().map(|c| c.to_string()).collect(),
        ..NetworkPolicy::default()
    }
}

fn lan_network() -> NetworkPolicy {
    network(&[&host_cidr(lan_ipv4())])
}

const BROAD: &[&str] = &["0.0.0.0/0", "::/0"];

fn http(url: impl Into<String>) -> Check {
    Check {
        url: Some(url.into()),
        timeout_seconds: Some(3),
        ..Check::default()
    }
}

fn url(addr: SocketAddr, scheme: &str, path: &str) -> String {
    format!("{scheme}://{addr}{path}")
}

async fn probe(check: &Check, network: &NetworkPolicy) -> ProbeResult {
    probe_with_secrets(check, network, Path::new("/nonexistent-secrets")).await
}

async fn probe_with_secrets(check: &Check, network: &NetworkPolicy, root: &Path) -> ProbeResult {
    vesper_probe::run(check, &CheckDefaults::default(), network, root).await
}

#[derive(Clone)]
enum Reply {
    Raw(Vec<u8>),
    /// Accept the request and never answer.
    Hang,
    /// Send a head, then trickle body bytes forever.
    Drip(Vec<u8>),
}

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    out.push_str(body);
    Reply::Raw(out.into_bytes())
}

type Responder = Arc<dyn Fn(&str) -> Reply + Send + Sync>;

/// A test server that records how many connections it accepted and the
/// request heads it received.
struct Server {
    addr: SocketAddr,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    async fn start(ip: IpAddr, reply: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        Self::start_with(ip, None, Arc::new(reply)).await
    }

    async fn start_tls(
        ip: IpAddr,
        acceptor: TlsAcceptor,
        reply: impl Fn(&str) -> Reply + Send + Sync + 'static,
    ) -> Self {
        Self::start_with(ip, Some(acceptor), Arc::new(reply)).await
    }

    async fn start_with(ip: IpAddr, tls: Option<TlsAcceptor>, reply: Responder) -> Self {
        let listener = TcpListener::bind(SocketAddr::new(ip, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (count, log) = (connections.clone(), requests.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let (tls, reply, log) = (tls.clone(), reply.clone(), log.clone());
                tokio::spawn(async move {
                    match tls {
                        Some(acceptor) => {
                            if let Ok(stream) = acceptor.accept(stream).await {
                                serve(stream, reply, log).await
                            }
                        }
                        None => serve(stream, reply, log).await,
                    }
                });
            }
        });
        Self {
            addr,
            connections,
            requests,
        }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    reply: Responder,
    log: Arc<Mutex<Vec<String>>>,
) {
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 65536 {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    log.lock().unwrap().push(head.clone());
    match reply(&head) {
        Reply::Raw(bytes) => {
            let _ = stream.write_all(&bytes).await;
            let _ = stream.shutdown().await;
        }
        Reply::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
        Reply::Drip(head) => {
            let _ = stream.write_all(&head).await;
            loop {
                if stream.write_all(b"x").await.is_err() {
                    return;
                }
                let _ = stream.flush().await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn secrets_dir(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vesper-probe-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
    }
    dir
}

fn assert_unhealthy(result: &ProbeResult, needle: &str) {
    assert!(!result.healthy, "expected failure, got {result:?}");
    assert!(
        result.message.contains(needle),
        "message {:?} should contain {needle:?}",
        result.message
    );
}

// ---------------------------------------------------------------- HTTP basics

#[tokio::test]
async fn http_status_and_body_substring_on_lan() {
    let ip = lan_ipv4();
    let server = Server::start(ip, |_| response("200 OK", &[], "status: vesper-ready\n")).await;

    let ok = probe(
        &http(url(server.addr, "http", "/health?x=1")),
        &lan_network(),
    )
    .await;
    assert!(ok.healthy, "{ok:?}");
    assert_eq!(ok.message, format!("HTTP 200 from {}", server.addr));

    let mut check = http(url(server.addr, "http", "/health"));
    check.body_contains = Some("vesper-ready".into());
    let matched = probe(&check, &lan_network()).await;
    assert!(matched.healthy, "{matched:?}");
    assert!(matched.message.ends_with("body matched"));

    check.body_contains = Some("vesper-broken".into());
    let missing = probe(&check, &lan_network()).await;
    assert_unhealthy(&missing, "did not contain the expected text");
    assert!(!missing.message.contains("status:"));

    let head = &server.requests()[0];
    assert!(head.starts_with("GET /health?x=1 HTTP/1.1\r\n"), "{head}");
    assert_eq!(
        header_value(head, "host"),
        Some(server.addr.to_string().as_str())
    );
}

#[tokio::test]
async fn unexpected_status_never_echoes_the_body() {
    let server = Server::start(lan_ipv4(), |_| {
        response(
            "503 Service Unavailable",
            &[],
            "<script>leak-me-body</script>",
        )
    })
    .await;
    let result = probe(&http(url(server.addr, "http", "/")), &lan_network()).await;
    assert_unhealthy(&result, "expected HTTP 200, got 503");
    assert!(!result.message.contains("leak-me-body"));

    let mut expect_503 = http(url(server.addr, "http", "/"));
    expect_503.expected_status = 503;
    assert!(probe(&expect_503, &lan_network()).await.healthy);
}

// ---------------------------------------------------------------- Bounds

#[tokio::test]
async fn body_reads_are_bounded() {
    let ip = lan_ipv4();
    let mut policy = lan_network();
    policy.max_response_bytes = 1000;

    // Declared length over the limit fails before reading.
    let declared = Server::start(ip, |_| response("200 OK", &[], &"a".repeat(1001))).await;
    let mut check = http(url(declared.addr, "http", "/"));
    check.body_contains = Some("a".into());
    assert_unhealthy(&probe(&check, &policy).await, "exceeded 1000 bytes");

    // Chunked body without a length is cut off at the limit too.
    let chunked = Server::start(ip, |_| {
        let chunk = "b".repeat(600);
        Reply::Raw(
            format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n258\r\n{chunk}\r\n258\r\n{chunk}\r\n0\r\n\r\n"
            )
            .into_bytes(),
        )
    })
    .await;
    let mut check = http(url(chunked.addr, "http", "/"));
    check.body_contains = Some("needle".into());
    assert_unhealthy(&probe(&check, &policy).await, "exceeded 1000 bytes");

    // Exactly the limit is accepted.
    let exact = Server::start(ip, |_| {
        response("200 OK", &[], &format!("{}needle", "c".repeat(994)))
    })
    .await;
    let mut check = http(url(exact.addr, "http", "/"));
    check.body_contains = Some("needle".into());
    assert!(probe(&check, &policy).await.healthy);

    // Status-only checks do not read (or bound) the body.
    let mut status_only = http(url(declared.addr, "http", "/"));
    status_only.body_contains = None;
    assert!(probe(&status_only, &policy).await.healthy);
}

#[tokio::test]
async fn oversized_response_heads_are_rejected() {
    let server = Server::start(lan_ipv4(), |_| {
        let big = "v".repeat(100 * 1024);
        response("200 OK", &[("X-Big", big.as_str())], "")
    })
    .await;
    let result = probe(&http(url(server.addr, "http", "/")), &lan_network()).await;
    assert_unhealthy(&result, "message head is too large");
    assert!(!result.message.contains("vvvv"));
}

#[tokio::test]
async fn total_timeout_bounds_silent_and_dripping_servers() {
    let ip = lan_ipv4();
    let silent = Server::start(ip, |_| Reply::Hang).await;
    let drip = Server::start(ip, |_| {
        Reply::Drip(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n".to_vec())
    })
    .await;

    for (check, label) in [
        (http(url(silent.addr, "http", "/")), "silent"),
        (
            Check {
                body_contains: Some("never".into()),
                ..http(url(drip.addr, "http", "/"))
            },
            "drip",
        ),
    ] {
        let check = Check {
            timeout_seconds: Some(1),
            ..check
        };
        let started = Instant::now();
        let result = probe(&check, &lan_network()).await;
        let elapsed = started.elapsed();
        assert_unhealthy(&result, "timed out after 1s");
        assert!(
            elapsed >= Duration::from_millis(950) && elapsed < Duration::from_millis(1800),
            "{label} took {elapsed:?}"
        );
        assert!(result.latency_ms >= 950, "{result:?}");
    }
}

// ---------------------------------------------------------------- TCP

#[tokio::test]
async fn tcp_connect_success_refusal_and_denial() {
    let ip = lan_ipv4();
    let listener = TcpListener::bind(SocketAddr::new(ip, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tcp = |host: String, port: u16| Check {
        kind: CheckKind::Tcp,
        host: Some(host),
        port: Some(port),
        timeout_seconds: Some(2),
        ..Check::default()
    };

    let ok = probe(&tcp(ip.to_string(), addr.port()), &lan_network()).await;
    assert!(ok.healthy, "{ok:?}");
    assert_eq!(ok.message, format!("TCP connect to {addr} succeeded"));
    listener.accept().await.unwrap();

    drop(listener);
    let refused = probe(&tcp(ip.to_string(), addr.port()), &lan_network()).await;
    assert_unhealthy(&refused, "connection refused");

    let loopback = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = loopback.local_addr().unwrap().port();
    for host in ["127.0.0.1", "localhost", "[::ffff:127.0.0.1]"] {
        let denied = probe(&tcp(host.into(), port), &network(BROAD)).await;
        assert_unhealthy(&denied, "denied by network policy");
    }
    let accepted = tokio::time::timeout(Duration::from_millis(100), loopback.accept()).await;
    assert!(
        accepted.is_err(),
        "loopback listener must never be contacted"
    );

    assert_unhealthy(
        &probe(&tcp(ip.to_string(), 0), &lan_network()).await,
        "no port",
    );
    assert_unhealthy(
        &probe(&tcp("bad host".into(), 22), &lan_network()).await,
        "not a valid hostname",
    );
}

// ---------------------------------------------------------------- Destination policy

#[tokio::test]
async fn always_denied_destinations_are_never_contacted() {
    let v4 = Server::start("127.0.0.1".parse().unwrap(), |_| {
        response("200 OK", &[], "")
    })
    .await;
    let v6 = Server::start("::1".parse().unwrap(), |_| response("200 OK", &[], "")).await;
    let (p4, p6) = (v4.addr.port(), v6.addr.port());
    let urls = [
        format!("http://127.0.0.1:{p4}/"),
        format!("http://127.1:{p4}/"),
        format!("http://2130706433:{p4}/"),
        format!("http://0x7f.0.0.1:{p4}/"),
        format!("http://localhost:{p4}/"),
        format!("http://LocalHost:{p4}/"),
        format!("http://[::ffff:127.0.0.1]:{p4}/"),
        format!("http://[::ffff:7f00:1]:{p4}/"),
        format!("http://[::1]:{p6}/"),
        format!("http://0.0.0.0:{p4}/"),
        format!("http://[::]:{p6}/"),
        "http://169.254.169.254/latest/meta-data/".into(),
        "http://[fd00:ec2::254]/".into(),
        "http://100.100.100.200/".into(),
        "http://[fe80::1]/".into(),
        "http://224.0.0.1/".into(),
        "http://255.255.255.255/".into(),
    ];
    for target in urls {
        let result = probe(&http(target.clone()), &network(BROAD)).await;
        assert_unhealthy(&result, "denied by network policy");
    }
    assert_eq!(v4.connections(), 0);
    assert_eq!(v6.connections(), 0);
}

#[tokio::test]
async fn allowlist_is_deny_by_default_and_narrow() {
    let ip = lan_ipv4();
    let server = Server::start(ip, |_| response("200 OK", &[], "")).await;
    let target = url(server.addr, "http", "/");

    assert_unhealthy(
        &probe(&http(&target), &network(&[])).await,
        "not in allowed_cidrs",
    );
    let IpAddr::V4(v4) = ip else { unreachable!() };
    let [a, b, c, d] = v4.octets();
    let neighbour = format!("{a}.{b}.{c}.{}/32", d ^ 1);
    assert_unhealthy(
        &probe(&http(&target), &network(&[&neighbour])).await,
        "not in allowed_cidrs",
    );
    assert_eq!(server.connections(), 0);

    assert_unhealthy(
        &probe(&http(&target), &network(&["not-a-cidr"])).await,
        "invalid CIDR",
    );
    assert!(
        probe(&http(&target), &network(&[&format!("{a}.{b}.0.0/16")]))
            .await
            .healthy
    );
}

#[tokio::test]
async fn ipv6_lan_destination_when_configured() {
    let Ok(raw) = std::env::var("VESPER_TEST_LAN_IPV6") else {
        eprintln!("skipping: VESPER_TEST_LAN_IPV6 not set");
        return;
    };
    let ip: IpAddr = raw.parse().unwrap();
    let server = Server::start(ip, |_| response("204 No Content", &[], "")).await;
    let mut check = http(url(server.addr, "http", "/"));
    check.expected_status = 204;
    let ok = probe(&check, &network(&[&host_cidr(ip)])).await;
    assert!(ok.healthy, "{ok:?}");
    assert_eq!(
        header_value(&server.requests()[0], "host"),
        Some(server.addr.to_string().as_str())
    );
    let denied = probe(&check, &lan_network()).await;
    assert_unhealthy(&denied, "not in allowed_cidrs");
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn urls_with_credentials_or_other_schemes_are_rejected() {
    let server = Server::start(lan_ipv4(), |_| response("200 OK", &[], "")).await;
    let addr = server.addr;
    let with_userinfo = probe(
        &http(format!("http://admin:hunter2@{addr}/")),
        &lan_network(),
    )
    .await;
    assert_unhealthy(&with_userinfo, "must not contain credentials");
    assert!(!with_userinfo.message.contains("hunter2"));
    assert_unhealthy(
        &probe(&http(format!("ftp://{addr}/")), &lan_network()).await,
        "only http",
    );
    assert_unhealthy(
        &probe(&http("not a url"), &lan_network()).await,
        "url is invalid",
    );
    assert_unhealthy(
        &probe(
            &Check {
                url: None,
                ..Check::default()
            },
            &lan_network(),
        )
        .await,
        "no url",
    );
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn push_checks_are_not_actively_probed() {
    let check = Check {
        kind: CheckKind::Push,
        ..Check::default()
    };
    assert_unhealthy(
        &probe(&check, &network(BROAD)).await,
        "cannot be actively probed",
    );
}

#[tokio::test]
async fn ambient_proxy_settings_are_ignored() {
    let ip = lan_ipv4();
    let proxy = Server::start(ip, |_| response("502 Bad Gateway", &[], "")).await;
    let target = Server::start(ip, |_| response("200 OK", &[], "")).await;
    let proxy_url = format!("http://{}", proxy.addr);
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        std::env::set_var(name, &proxy_url);
    }
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");
    let result = probe(&http(url(target.addr, "http", "/")), &lan_network()).await;
    assert!(result.healthy, "{result:?}");
    assert_eq!(proxy.connections(), 0);
    assert_eq!(target.connections(), 1);
}

// ---------------------------------------------------------------- Redirects and secrets

fn redirect_to(location: String) -> impl Fn(&str) -> Reply + Send + Sync + 'static {
    move |_| response("302 Found", &[("Location", location.as_str())], "")
}

fn bearer_check(target: String, env: &str, secret: &str) -> Check {
    std::env::set_var(env, secret);
    let mut check = http(target);
    check
        .headers
        .insert("Authorization".into(), SecretRef::Env(env.into()));
    check.follow_redirects = true;
    check
}

#[tokio::test]
async fn redirects_are_off_by_default() {
    let ip = lan_ipv4();
    let dest = Server::start(ip, |_| response("200 OK", &[], "")).await;
    let start = Server::start(ip, redirect_to(url(dest.addr, "http", "/"))).await;

    let result = probe(&http(url(start.addr, "http", "/")), &lan_network()).await;
    assert_unhealthy(&result, "expected HTTP 200, got 302");
    let mut expect_redirect = http(url(start.addr, "http", "/"));
    expect_redirect.expected_status = 302;
    assert!(probe(&expect_redirect, &lan_network()).await.healthy);
    assert_eq!(dest.connections(), 0);
}

#[tokio::test]
async fn secrets_follow_same_origin_redirects_only() {
    let ip = lan_ipv4();
    let secret = "Bearer s3cr3t-token-value";

    let same = Server::start(ip, |head| {
        if head.starts_with("GET /start ") {
            response("301 Moved Permanently", &[("Location", "/next")], "")
        } else {
            response("200 OK", &[], "")
        }
    })
    .await;
    let check = bearer_check(
        url(same.addr, "http", "/start"),
        "VESPER_PROBE_TEST_SAME",
        secret,
    );
    let result = probe(&check, &lan_network()).await;
    assert!(result.healthy, "{result:?}");
    assert!(result.message.ends_with("after 1 redirect"));
    let heads = same.requests();
    assert_eq!(heads.len(), 2);
    for head in &heads {
        assert_eq!(header_value(head, "authorization"), Some(secret));
    }
    assert!(!result.message.contains("s3cr3t"));
}

#[tokio::test]
async fn secrets_never_cross_origins_even_when_coming_back() {
    let ip = lan_ipv4();
    let secret = "Bearer cross-origin-secret";
    // origin A /start -> origin B -> back to origin A /back
    let a_addr = Arc::new(std::sync::OnceLock::<SocketAddr>::new());
    let back = a_addr.clone();
    let other = Server::start(ip, move |_| {
        let location = format!("http://{}/back", back.get().unwrap());
        response("302 Found", &[("Location", location.as_str())], "")
    })
    .await;
    let other_location = url(other.addr, "http", "/elsewhere");
    let origin = Server::start(ip, move |head| {
        if head.starts_with("GET /start ") {
            response(
                "307 Temporary Redirect",
                &[("Location", other_location.as_str())],
                "",
            )
        } else {
            response("200 OK", &[], "")
        }
    })
    .await;
    a_addr.set(origin.addr).unwrap();

    let check = bearer_check(
        url(origin.addr, "http", "/start"),
        "VESPER_PROBE_TEST_CROSS",
        secret,
    );
    let result = probe(&check, &lan_network()).await;
    assert!(result.healthy, "{result:?}");
    assert!(result.message.ends_with("after 2 redirects"), "{result:?}");

    let origin_heads = origin.requests();
    assert_eq!(origin_heads.len(), 2);
    assert_eq!(
        header_value(&origin_heads[0], "authorization"),
        Some(secret)
    );
    assert!(origin_heads[1].starts_with("GET /back "));
    assert_eq!(header_value(&origin_heads[1], "authorization"), None);
    let other_heads = other.requests();
    assert_eq!(other_heads.len(), 1);
    assert!(!other_heads[0].contains("cross-origin-secret"));
}

#[tokio::test]
async fn redirects_are_revalidated_against_policy() {
    let ip = lan_ipv4();
    let loopback = Server::start("127.0.0.1".parse().unwrap(), |_| {
        response("200 OK", &[], "")
    })
    .await;
    let lport = loopback.addr.port();

    for (location, broad, reason) in [
        (url(loopback.addr, "http", "/"), true, "loopback"),
        (format!("http://localhost:{lport}/"), true, "loopback"),
        (
            format!("http://[::ffff:127.0.0.1]:{lport}/"),
            true,
            "loopback",
        ),
        ("http://169.254.169.254/".into(), true, "cloud metadata"),
        ("http://192.0.2.10/".into(), false, "not in allowed_cidrs"),
        ("file:///etc/passwd".into(), true, "only http"),
        (
            "http://user:pw@example.invalid/".into(),
            true,
            "credentials",
        ),
    ] {
        let start = Server::start(ip, redirect_to(location)).await;
        let mut policy = if broad { network(BROAD) } else { network(&[]) };
        policy.allowed_cidrs.push(host_cidr(ip));
        let mut check = http(url(start.addr, "http", "/"));
        check.follow_redirects = true;
        let result = probe(&check, &policy).await;
        assert_unhealthy(&result, reason);
        assert!(result.message.contains("refused redirect") || result.message.contains("denied"));
        assert_eq!(start.connections(), 1);
    }
    assert_eq!(loopback.connections(), 0);
}

#[tokio::test]
async fn redirect_chains_are_bounded() {
    let server = Server::start(lan_ipv4(), |_| {
        response("302 Found", &[("Location", "/again")], "")
    })
    .await;
    let mut check = http(url(server.addr, "http", "/"));
    check.follow_redirects = true;
    let result = probe(&check, &lan_network()).await;
    assert_unhealthy(&result, "stopped after 5 redirects");
    assert_eq!(server.requests().len(), 6);

    let missing = Server::start(lan_ipv4(), |_| response("302 Found", &[], "")).await;
    let mut check = http(url(missing.addr, "http", "/"));
    check.follow_redirects = true;
    assert_unhealthy(
        &probe(&check, &lan_network()).await,
        "missing or invalid Location",
    );
}

#[tokio::test]
async fn secret_values_and_resolution_errors_stay_out_of_messages() {
    let ip = lan_ipv4();
    let server = Server::start(ip, |head| {
        // A hostile server reflecting the credential back must not leak it.
        let auth = header_value(head, "x-api-key").unwrap_or("").to_string();
        response(
            "500 Internal Server Error",
            &[("X-Echo", auth.as_str())],
            &auth,
        )
    })
    .await;
    let root = secrets_dir(&[("api-key", "file-secret-value\n")]);
    let mut check = http(url(server.addr, "http", "/"));
    check
        .headers
        .insert("X-Api-Key".into(), SecretRef::File("api-key".into()));
    let result = probe_with_secrets(&check, &lan_network(), &root).await;
    assert_unhealthy(&result, "got 500");
    assert!(!result.message.contains("file-secret-value"));
    assert_eq!(
        header_value(&server.requests()[0], "x-api-key"),
        Some("file-secret-value"),
        "one trailing newline is stripped"
    );

    let mut unresolved = http(url(server.addr, "http", "/"));
    unresolved.headers.insert(
        "Authorization".into(),
        SecretRef::Env("VESPER_PROBE_TEST_UNSET".into()),
    );
    std::env::remove_var("VESPER_PROBE_TEST_UNSET");
    let result = probe(&unresolved, &lan_network()).await;
    assert_unhealthy(&result, "header `authorization`");
    assert_eq!(
        server.connections(),
        1,
        "unresolved secrets fail before connecting"
    );

    let mut reserved = http(url(server.addr, "http", "/"));
    reserved.headers.insert(
        "Host".into(),
        SecretRef::Env("VESPER_PROBE_TEST_SAME".into()),
    );
    assert_unhealthy(
        &probe(&reserved, &lan_network()).await,
        "cannot be configured",
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------- TLS

struct Pki {
    ca_pem: String,
    acceptor: TlsAcceptor,
}

fn make_pki(ip: IpAddr, dns: &str) -> Pki {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, SanType};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Vesper Test CA");
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);

    let leaf_key = KeyPair::generate().unwrap();
    let mut leaf_params = CertificateParams::new(vec![dns.to_string()]).unwrap();
    leaf_params.subject_alt_names.push(SanType::IpAddress(ip));
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        rustls_pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
    )
    .unwrap();
    Pki {
        ca_pem: ca_cert.pem(),
        acceptor: TlsAcceptor::from(Arc::new(config)),
    }
}

#[tokio::test]
async fn tls_is_verified_against_public_roots_and_the_private_ca() {
    let ip = lan_ipv4();
    let pki = make_pki(ip, "probe.test");
    let server = Server::start_tls(ip, pki.acceptor.clone(), |_| {
        response("200 OK", &[], "tls-ok")
    })
    .await;
    let target = url(server.addr, "https", "/");
    let root = secrets_dir(&[("ca.pem", &pki.ca_pem), ("junk.pem", "not a certificate")]);

    let untrusted = probe_with_secrets(&http(&target), &lan_network(), &root).await;
    assert_unhealthy(&untrusted, "invalid peer certificate");

    let mut trusted_policy = lan_network();
    trusted_policy.private_ca_file = Some("ca.pem".into());
    let mut check = http(&target);
    check.body_contains = Some("tls-ok".into());
    let trusted = probe_with_secrets(&check, &trusted_policy, &root).await;
    assert!(trusted.healthy, "{trusted:?}");

    // A certificate valid for another address is rejected even with the CA.
    let other = make_pki("10.9.9.9".parse().unwrap(), "other.test");
    let root_other = secrets_dir(&[("ca.pem", &other.ca_pem)]);
    let wrong_name = Server::start_tls(ip, other.acceptor, |_| response("200 OK", &[], "")).await;
    let mismatch = probe_with_secrets(
        &http(url(wrong_name.addr, "https", "/")),
        &trusted_policy,
        &root_other,
    )
    .await;
    assert_unhealthy(&mismatch, "invalid peer certificate");

    for (file, reason) in [
        ("junk.pem", "no certificates"),
        ("missing.pem", "private CA"),
        ("../ca.pem", "private CA"),
    ] {
        let mut policy = lan_network();
        policy.private_ca_file = Some(file.into());
        assert_unhealthy(
            &probe_with_secrets(&http(&target), &policy, &root).await,
            reason,
        );
    }
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(root_other).unwrap();
}

#[tokio::test]
async fn https_to_http_redirect_downgrade_is_refused() {
    let ip = lan_ipv4();
    let pki = make_pki(ip, "probe.test");
    let plain = Server::start(ip, |_| response("200 OK", &[], "")).await;
    let location = format!("http://{}/", plain.addr);
    let secure = Server::start_tls(ip, pki.acceptor, redirect_to(location)).await;
    let root = secrets_dir(&[("ca.pem", &pki.ca_pem)]);
    let mut policy = lan_network();
    policy.private_ca_file = Some("ca.pem".into());
    let mut check = http(url(secure.addr, "https", "/"));
    check.follow_redirects = true;
    let result = probe_with_secrets(&check, &policy, &root).await;
    assert_unhealthy(&result, "refused redirect from HTTPS to HTTP");
    assert_eq!(plain.connections(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

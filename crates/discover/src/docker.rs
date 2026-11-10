//! A minimal, read-only Docker Engine API client. It sends exactly one
//! request, `GET /containers/json?all=true`, which returns names and labels
//! but not environment variables, and never writes to Docker.

use crate::labels::Container;
use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::{header, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

/// Where the Docker Engine API listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `unix:///var/run/docker.sock`: a local socket or a socket proxy's socket.
    Unix(PathBuf),
    /// `tcp://host:port`: plain HTTP, meant for a read-only socket proxy on a
    /// private network.
    Tcp(String),
}

impl std::str::FromStr for Endpoint {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        if let Some(path) = s.strip_prefix("unix://") {
            if !path.starts_with('/') {
                bail!("unix:// endpoints need an absolute socket path, such as unix:///var/run/docker.sock");
            }
            return Ok(Endpoint::Unix(path.into()));
        }
        if let Some(address) = s.strip_prefix("tcp://") {
            let valid = address.rsplit_once(':').is_some_and(|(host, port)| {
                !host.is_empty()
                    && !host.contains('/')
                    && !host.contains('@')
                    && port.parse::<u16>().is_ok_and(|p| p > 0)
            });
            if !valid {
                bail!("tcp:// endpoints are host:port, such as tcp://docker-socket-proxy:2375");
            }
            return Ok(Endpoint::Tcp(address.into()));
        }
        bail!("Docker endpoints start with unix:// or tcp://")
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Unix(path) => write!(f, "unix://{}", path.display()),
            Endpoint::Tcp(address) => write!(f, "tcp://{address}"),
        }
    }
}

#[derive(Deserialize)]
struct Summary {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Names", default)]
    names: Vec<String>,
    #[serde(rename = "Labels", default)]
    labels: Option<BTreeMap<String, String>>,
}

/// Every container on the host, running or not, with its labels. Stopped
/// containers are included so that a crashed container stays monitored.
pub async fn containers(endpoint: &Endpoint) -> Result<Vec<Container>> {
    let body = tokio::time::timeout(TIMEOUT, async {
        match endpoint {
            Endpoint::Unix(path) => get(tokio::net::UnixStream::connect(path).await?).await,
            Endpoint::Tcp(address) => get(tokio::net::TcpStream::connect(address).await?).await,
        }
    })
    .await
    .map_err(|_| anyhow!("no answer within {} seconds", TIMEOUT.as_secs()))??;
    let summaries: Vec<Summary> =
        serde_json::from_slice(&body).context("unexpected container list from Docker")?;
    Ok(summaries
        .into_iter()
        .map(|s| Container {
            name: s
                .names
                .first()
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| s.id.chars().take(12).collect()),
            labels: s.labels.unwrap_or_default(),
        })
        .collect())
}

async fn get<S>(stream: S) -> Result<Bytes>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .context("Docker API handshake failed")?;
    let connection = tokio::spawn(connection);
    let request = Request::get("/containers/json?all=true")
        .header(header::HOST, "docker")
        .body(Empty::<Bytes>::new())?;
    let result = async {
        let response = sender.send_request(request).await?;
        let status = response.status();
        let body = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
            .collect()
            .await
            .map_err(|_| anyhow!("container list is larger than {MAX_RESPONSE_BYTES} bytes or was cut off"))?
            .to_bytes();
        if status != StatusCode::OK {
            bail!("Docker answered HTTP {status} to GET /containers/json; a socket proxy must allow container listing");
        }
        Ok(body)
    }
    .await;
    connection.abort();
    result
}

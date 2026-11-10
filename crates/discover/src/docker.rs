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
            id: s.id,
            labels: s.labels.unwrap_or_default(),
        })
        .collect())
}

/// The ID, or ID prefix, of the container this process runs in: from the
/// files Docker and Podman bind-mount at /etc/hostname, /etc/hosts and
/// /etc/resolv.conf, or else from a default hostname, which is the ID's first
/// 12 characters. `None` outside a container.
pub fn own_container_id() -> Option<String> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    id_from_mountinfo(&mountinfo).or_else(|| {
        std::env::var("HOSTNAME")
            .ok()
            .filter(|h| h.len() == 12 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    })
}

fn id_from_mountinfo(mountinfo: &str) -> Option<String> {
    mountinfo.lines().find_map(|line| {
        // Fields: ID, parent ID, major:minor, root, mount point, ...
        let mut fields = line.split(' ').skip(3);
        let (root, mount_point) = (fields.next()?, fields.next()?);
        if !matches!(
            mount_point,
            "/etc/hostname" | "/etc/hosts" | "/etc/resolv.conf"
        ) {
            return None;
        }
        // …/containers/<id>/hostname (Docker) or
        // …/overlay-containers/<id>/userdata/hostname (Podman).
        let segments: Vec<&str> = root.split('/').collect();
        segments.windows(2).find_map(|w| {
            (w[0].ends_with("containers")
                && w[1].len() == 64
                && w[1].bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| w[1].to_string())
        })
    })
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

#[cfg(test)]
mod tests {
    #[test]
    fn own_container_id_comes_from_the_hostname_bind_mounts() {
        let id = "4f".repeat(32);
        let docker = format!(
            "1 0 0:31 / / rw - overlay overlay rw\n\
             2 1 254:1 /var/lib/docker/containers/{id}/resolv.conf /etc/resolv.conf rw - ext4 /dev/vda1 rw\n"
        );
        assert_eq!(super::id_from_mountinfo(&docker), Some(id.clone()));
        let podman = format!(
            "7 1 0:44 /containers/storage/overlay-containers/{id}/userdata/hostname /etc/hostname rw - tmpfs tmpfs rw\n"
        );
        assert_eq!(super::id_from_mountinfo(&podman), Some(id.clone()));
        let elsewhere = format!(
            "2 1 254:1 /var/lib/docker/containers/{id}/data /data rw - ext4 /dev/vda1 rw\n"
        );
        assert_eq!(super::id_from_mountinfo(&elsewhere), None);
    }
}

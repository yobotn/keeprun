//! HTTP health checks. An app can run and still not answer, e.g. a server stuck on a lock or a
//! dev server whose build hangs. The process state cannot show that; a request can.
//!
//! Plain HTTP/1.1 on the local machine only, so no client library and no TLS: the check goes to
//! the port the app listens on.

use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Where to send the check: a path on the app's own port, or a full `http://` URL.
#[derive(Debug, PartialEq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub path: String,
}

/// `/health` goes to the app's port. `http://localhost:5001/ready` names its own port.
pub fn target(check: &str, app_port: Option<u16>) -> Result<Target, String> {
    let check = check.trim();
    if let Some(rest) = check.strip_prefix("http://") {
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (
                h,
                p.parse::<u16>()
                    .map_err(|_| format!("\"{p}\" is not a port number."))?,
            ),
            _ => (authority, 80),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if !matches!(
            host.to_ascii_lowercase().as_str(),
            "localhost" | "127.0.0.1" | "::1"
        ) {
            return Err("Health checks go to this machine: use localhost, 127.0.0.1 or a path like /health.".into());
        }
        return Ok(Target {
            host: host.to_string(),
            port,
            path: path.to_string(),
        });
    }
    // Git Bash turns an argument like /health into C:/Program Files/Git/health.
    let b = check.as_bytes();
    if b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\') {
        return Err(format!(
            "\"{check}\" is a file path, not a URL path. Git Bash turns /health into a path: write //health there, or set MSYS_NO_PATHCONV=1."
        ));
    }
    if check.starts_with("https://") {
        return Err(
            "Health checks use plain HTTP on this machine. Use a path like /health.".into(),
        );
    }
    let port = app_port.ok_or("The app listens on no port yet.")?;
    let path = if check.starts_with('/') {
        check.to_string()
    } else {
        format!("/{check}")
    };
    Ok(Target {
        host: "127.0.0.1".into(),
        port,
        path,
    })
}

/// Result of one check, e.g. `Ok("200 in 12 ms")` or `Err("Timed out after 5 s")`.
pub async fn check(t: &Target) -> Result<String, String> {
    let start = Instant::now();
    let status = tokio::time::timeout(TIMEOUT, request(t))
        .await
        .map_err(|_| format!("No answer within {} s", TIMEOUT.as_secs()))??;
    let ms = start.elapsed().as_millis();
    if (200..400).contains(&status) {
        Ok(format!("{status} in {ms} ms"))
    } else {
        Err(format!("HTTP {status} from {}", t.path))
    }
}

async fn request(t: &Target) -> Result<u16, String> {
    // Servers bind IPv4, IPv6 or both. localhost tries both.
    let addrs: Vec<String> = match t.host.as_str() {
        "localhost" => vec![format!("127.0.0.1:{}", t.port), format!("[::1]:{}", t.port)],
        "::1" => vec![format!("[::1]:{}", t.port)],
        h => vec![format!("{h}:{}", t.port)],
    };
    let mut last = String::new();
    for addr in addrs {
        match TcpStream::connect(&addr).await {
            Ok(mut s) => {
                let req = format!(
                    "GET {} HTTP/1.1\r\nHost: localhost:{}\r\nUser-Agent: Keeprun health check\r\nAccept: */*\r\nConnection: close\r\n\r\n",
                    t.path, t.port
                );
                s.write_all(req.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 256];
                let mut got = 0;
                // The status line is all that matters.
                while got < buf.len() {
                    let n = s.read(&mut buf[got..]).await.map_err(|e| e.to_string())?;
                    if n == 0 || buf[..got + n].contains(&b'\n') {
                        got += n;
                        break;
                    }
                    got += n;
                }
                return status_code(&buf[..got]).ok_or_else(|| "Not an HTTP answer".into());
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(if last.contains("refused") || last.contains("10061") {
        format!("Nothing answers on port {}", t.port)
    } else {
        last
    })
}

fn status_code(head: &[u8]) -> Option<u16> {
    let line = std::str::from_utf8(head).ok()?.lines().next()?;
    let mut parts = line.split_whitespace();
    parts.next().filter(|p| p.starts_with("HTTP/"))?;
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_a_path_or_a_local_url() {
        assert_eq!(
            target("/health", Some(3000)).unwrap(),
            Target {
                host: "127.0.0.1".into(),
                port: 3000,
                path: "/health".into()
            }
        );
        assert_eq!(target("ready", Some(80)).unwrap().path, "/ready");
        assert_eq!(
            target("http://localhost:5001/api/ready", None).unwrap(),
            Target {
                host: "localhost".into(),
                port: 5001,
                path: "/api/ready".into()
            }
        );
        assert_eq!(target("http://[::1]:8080", None).unwrap().port, 8080);
        assert!(target("/health", None).is_err());
        assert!(target("http://example.com/health", None).is_err());
        assert!(target("https://localhost/health", None).is_err());
        assert!(target("C:/Program Files/Git/health", Some(80))
            .unwrap_err()
            .contains("Git Bash"));
    }

    #[test]
    fn reads_the_status_code() {
        assert_eq!(status_code(b"HTTP/1.1 204 No Content\r\n"), Some(204));
        assert_eq!(status_code(b"HTTP/1.0 503 Service Unavailable"), Some(503));
        assert_eq!(status_code(b"SSH-2.0-OpenSSH"), None);
    }

    async fn serve(answer: &'static str) -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(answer.as_bytes()).await;
            }
        });
        port
    }

    #[tokio::test]
    async fn checks_a_live_server() {
        let ok = serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
        assert!(check(&target("/health", Some(ok)).unwrap())
            .await
            .unwrap()
            .starts_with("200 in"));
        let bad = serve("HTTP/1.1 503 Service Unavailable\r\n\r\n").await;
        assert_eq!(
            check(&target("/health", Some(bad)).unwrap())
                .await
                .unwrap_err(),
            "HTTP 503 from /health"
        );
        // A port nobody listens on: bind one, then free it.
        let free = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert_eq!(
            check(&target("/health", Some(free)).unwrap())
                .await
                .unwrap_err(),
            format!("Nothing answers on port {free}")
        );
    }
}

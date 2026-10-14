// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Максимальный размер HTTP-заголовков (клиентских и ответа upstream на CONNECT).
const MAX_HEADERS_SIZE: usize = 65536;
/// Таймаут чтения HTTP-заголовков (от клиента и ответа upstream на CONNECT).
const HEADERS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Таймаут TCP-подключения к upstream.
const UPSTREAM_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub enum Upstream {
    Http {
        host: String,
        port: u16,
        username: String,
        password: String,
    },
    Socks5 {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
    Ssh {
        session: crate::proxy::ssh::SharedSession,
    },
    /// The SOCKS port of a tor instance. The relay holds the lease, so the
    /// instance lives as long as the relay and the connections it carries.
    Tor {
        lease: veydan_tor::Lease,
    },
}

/// Запускает локальный HTTP-прокси на 127.0.0.1:<random port>.
/// Принимает CONNECT и plain HTTP запросы от Firefox,
/// пробрасывает через HTTP или SOCKS5 upstream.
pub async fn spawn(upstream: Upstream) -> anyhow::Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    let local_port = local_addr.port();

    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

    let upstream = std::sync::Arc::new(upstream);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((client, peer)) => {
                            let up = std::sync::Arc::clone(&upstream);
                            tokio::spawn(async move {
                                if peer_of_another_user(local_addr, peer).await {
                                    eprintln!("local proxy: refused {peer}: a process of another user");
                                    return;
                                }
                                let _ = handle(client, up).await;
                            });
                        }
                        Err(e) => {
                            // Transient errors (ECONNABORTED, EMFILE, ...) must not
                            // kill the listener — log, back off briefly and retry.
                            eprintln!("local proxy: accept failed: {e}");
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    }
                }
                _ = &mut stop_rx => break,
            }
        }
    });

    Ok((local_port, stop_tx))
}

/// The relay has no password of its own (it speaks HTTP to the browser,
/// and Firefox gives an HTTP proxy none without asking the user), so any
/// local process that finds the port could send its traffic through the
/// user's proxy, SSH or tor.
/// On Linux a connection from a process of another user is refused: the
/// owner of the other end is read from `/proc/net/tcp`. A process of the same
/// user is not told apart (it can read the profile's `user.js` anyway), and
/// on Windows and macOS the port is open to every local user (platform-spec
/// 5.11).
#[cfg(target_os = "linux")]
async fn peer_of_another_user(local: std::net::SocketAddr, peer: std::net::SocketAddr) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (std::net::SocketAddr::V4(local), std::net::SocketAddr::V4(peer)) = (local, peer) else {
        return false;
    };
    let Ok(me) = tokio::fs::metadata("/proc/self").await.map(|m| m.uid()) else {
        return false;
    };
    let Ok(table) = tokio::fs::read_to_string("/proc/net/tcp").await else {
        return false;
    };
    // Not found (the client is gone already): nothing to refuse.
    tcp_peer_uid(&table, local, peer).is_some_and(|uid| uid != me)
}

#[cfg(not(target_os = "linux"))]
async fn peer_of_another_user(_local: std::net::SocketAddr, _peer: std::net::SocketAddr) -> bool {
    false
}

/// The uid of the client socket of a loopback connection in the text of
/// `/proc/net/tcp`: the row whose local address is `peer` and remote address
/// is `local` (the relay's own socket is the row the other way round).
#[cfg(target_os = "linux")]
fn tcp_peer_uid(table: &str, local: std::net::SocketAddrV4, peer: std::net::SocketAddrV4) -> Option<u32> {
    // The kernel prints the address as the u32 it holds in network order.
    let key = |a: std::net::SocketAddrV4| {
        format!("{:08X}:{:04X}", u32::from_ne_bytes(a.ip().octets()), a.port())
    };
    let (client, relay) = (key(peer), key(local));
    table.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        (f.len() > 7 && f[1] == client && f[2] == relay)
            .then(|| f[7].parse().ok())
            .flatten()
    })
}

async fn handle(client: TcpStream, upstream: std::sync::Arc<Upstream>) -> anyhow::Result<()> {
    match upstream.as_ref() {
        Upstream::Http {
            host,
            port,
            username,
            password,
        } => handle_http_upstream(client, host, *port, username, password).await,
        Upstream::Socks5 {
            host,
            port,
            username,
            password,
        } => {
            handle_socks5_upstream(
                client,
                host,
                *port,
                username.as_deref(),
                password.as_deref(),
            )
            .await
        }
        Upstream::Ssh { session } => handle_ssh_upstream(client, session).await,
        // The credentials of the lease are what tor tells consumers apart by.
        Upstream::Tor { lease } => {
            handle_socks5_upstream(
                client,
                "127.0.0.1",
                lease.socks_port,
                Some(&lease.username),
                Some(&lease.password),
            )
            .await
        }
    }
}

// ── HTTP upstream ─────────────────────────────────────────────────────────────

async fn handle_http_upstream(
    mut client: TcpStream,
    upstream_host: &str,
    upstream_port: u16,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let (headers_str, body) = read_http_headers(&mut client).await?;

    let auth_b64 =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    let auth_line = format!("Proxy-Authorization: Basic {}", auth_b64);
    // Plain HTTP: форсируем Connection: close, чтобы клиент открывал новое
    // соединение на каждый запрос — после первого запроса мы уходим в raw relay
    // и не смогли бы добавить Proxy-Authorization в последующие запросы.
    let request_head = if headers_str.starts_with("CONNECT ") {
        headers_str.clone()
    } else {
        force_connection_close(&headers_str)
    };
    let modified = inject_after_first_line(&request_head, &auth_line);

    let mut server = connect_upstream(upstream_host, upstream_port).await?;
    server.write_all(modified.as_bytes()).await?;
    server.write_all(b"\r\n\r\n").await?;

    relay_after_connect(&mut client, &mut server, &headers_str, &body, true).await
}

// ── SOCKS5 upstream ───────────────────────────────────────────────────────────

async fn handle_socks5_upstream(
    mut client: TcpStream,
    socks_host: &str,
    socks_port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<()> {
    let (headers_str, body) = read_http_headers(&mut client).await?;

    let is_connect = headers_str.starts_with("CONNECT ");

    let (target_host, target_port) = if is_connect {
        parse_connect_target(&headers_str).ok_or_else(|| anyhow::anyhow!("bad CONNECT line"))?
    } else {
        parse_http_target(&headers_str).ok_or_else(|| anyhow::anyhow!("bad HTTP request line"))?
    };

    let mut server = socks5_connect(
        socks_host,
        socks_port,
        &target_host,
        target_port,
        username,
        password,
    )
    .await?;

    if is_connect {
        client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await?;
        // Клиент мог прислать байты (например, TLS ClientHello) сразу за CONNECT
        if !body.is_empty() {
            server.write_all(&body).await?;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
    } else {
        // Plain HTTP: перепишем первую строку в relative path и форвардим.
        // Connection: close — см. комментарий в handle_http_upstream.
        let relative = force_connection_close(&rewrite_to_relative(&headers_str));
        server.write_all(relative.as_bytes()).await?;
        server.write_all(b"\r\n\r\n").await?;
        if !body.is_empty() {
            server.write_all(&body).await?;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
    }

    Ok(())
}

/// Устанавливает SOCKS5 соединение через прокси к target_host:target_port.
pub async fn socks5_connect(
    proxy_host: &str,
    proxy_port: u16,
    target_host: &str,
    target_port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<TcpStream> {
    anyhow::ensure!(
        target_host.len() <= 255,
        "SOCKS5: target host too long (max 255 bytes)"
    );

    let mut s = connect_upstream(proxy_host, proxy_port).await?;

    let has_auth = username.map(|u| !u.is_empty()).unwrap_or(false);

    // Предлагаем методы аутентификации. With credentials only user/pass is
    // offered: a server (or someone in the path) answering "no auth" would
    // otherwise carry the traffic without ever seeing them, and tor tells
    // its consumers apart by them.
    if has_auth {
        s.write_all(&[0x05, 0x01, 0x02]).await?; // user/pass only
    } else {
        s.write_all(&[0x05, 0x01, 0x00]).await?; // no-auth only
    }

    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await?;
    anyhow::ensure!(resp[0] == 0x05, "SOCKS5: bad version");

    match resp[1] {
        0x00 if has_auth => anyhow::bail!("SOCKS5: the server skipped the authentication it was asked for"),
        0x00 => {} // no auth — ok
        0x02 => {
            // username/password auth (RFC 1929)
            let user = username.unwrap_or("");
            let pass = password.unwrap_or("");
            anyhow::ensure!(
                user.len() <= 255 && pass.len() <= 255,
                "SOCKS5: username/password too long (max 255 bytes)"
            );
            let mut req = vec![0x01u8];
            req.push(user.len() as u8);
            req.extend_from_slice(user.as_bytes());
            req.push(pass.len() as u8);
            req.extend_from_slice(pass.as_bytes());
            s.write_all(&req).await?;
            let mut ar = [0u8; 2];
            s.read_exact(&mut ar).await?;
            anyhow::ensure!(ar[1] == 0x00, "SOCKS5: auth failed");
        }
        0xFF => anyhow::bail!("SOCKS5: no acceptable auth method"),
        m => anyhow::bail!("SOCKS5: unknown auth method 0x{:02x}", m),
    }

    // CONNECT через DOMAINNAME (ATYP=0x03); an IPv6 address as itself
    // (ATYP=0x04): `::1` is no domain name.
    let mut req = vec![0x05, 0x01, 0x00];
    if let Ok(ip) = target_host.parse::<std::net::Ipv6Addr>() {
        req.push(0x04);
        req.extend_from_slice(&ip.octets());
    } else {
        req.push(0x03);
        req.push(target_host.len() as u8);
        req.extend_from_slice(target_host.as_bytes());
    }
    req.push((target_port >> 8) as u8);
    req.push((target_port & 0xff) as u8);
    s.write_all(&req).await?;

    let mut hdr = [0u8; 4];
    s.read_exact(&mut hdr).await?;
    anyhow::ensure!(hdr[0] == 0x05, "SOCKS5: bad reply version");
    anyhow::ensure!(
        hdr[1] == 0x00,
        "SOCKS5: connect failed (code=0x{:02x})",
        hdr[1]
    );

    // Пропускаем bound address из ответа
    match hdr[3] {
        0x01 => {
            let mut skip = [0u8; 6];
            s.read_exact(&mut skip).await?;
        }
        0x03 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            let mut skip = vec![0u8; l[0] as usize + 2];
            s.read_exact(&mut skip).await?;
        }
        0x04 => {
            let mut skip = [0u8; 18];
            s.read_exact(&mut skip).await?;
        }
        _ => anyhow::bail!("SOCKS5: unknown ATYP in reply"),
    }

    Ok(s)
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

/// TCP-подключение к upstream с таймаутом.
async fn connect_upstream(host: &str, port: u16) -> anyhow::Result<TcpStream> {
    match tokio::time::timeout(
        UPSTREAM_CONNECT_TIMEOUT,
        TcpStream::connect(format!("{}:{}", host, port)),
    )
    .await
    {
        Ok(res) => Ok(res?),
        Err(_) => anyhow::bail!("connect to upstream {}:{} timed out", host, port),
    }
}

async fn read_http_headers(stream: &mut TcpStream) -> anyhow::Result<(String, Vec<u8>)> {
    tokio::time::timeout(HEADERS_TIMEOUT, async {
        let mut buf = Vec::with_capacity(4096);
        let header_end = loop {
            let mut tmp = [0u8; 2048];
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                anyhow::bail!("connection closed before headers");
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_header_end(&buf) {
                break pos;
            }
            anyhow::ensure!(buf.len() <= MAX_HEADERS_SIZE, "headers too large");
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let body = buf[header_end + 4..].to_vec();
        Ok((headers, body))
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out reading request headers"))?
}

async fn relay_after_connect(
    client: &mut TcpStream,
    server: &mut TcpStream,
    headers_str: &str,
    body: &[u8],
    _is_http_upstream: bool,
) -> anyhow::Result<()> {
    if headers_str.starts_with("CONNECT ") {
        // Читаем ответ прокси на CONNECT (с таймаутом и лимитом размера)
        let mut resp_buf = Vec::with_capacity(512);
        tokio::time::timeout(HEADERS_TIMEOUT, async {
            loop {
                let mut tmp = [0u8; 512];
                let n = server.read(&mut tmp).await?;
                if n == 0 {
                    break;
                }
                resp_buf.extend_from_slice(&tmp[..n]);
                if find_header_end(&resp_buf).is_some() {
                    break;
                }
                anyhow::ensure!(
                    resp_buf.len() <= MAX_HEADERS_SIZE,
                    "upstream CONNECT response too large"
                );
            }
            anyhow::Ok(())
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out reading upstream CONNECT response"))??;
        client.write_all(&resp_buf).await?;
        if !connect_response_ok(&resp_buf) {
            return Ok(());
        }
        // Клиент мог прислать байты (например, TLS ClientHello) сразу за CONNECT
        if !body.is_empty() {
            server.write_all(body).await?;
        }
        let _ = tokio::io::copy_bidirectional(client, server).await;
    } else {
        if !body.is_empty() {
            server.write_all(body).await?;
        }
        let _ = tokio::io::copy_bidirectional(client, server).await;
    }
    Ok(())
}

/// Проверяет статусную строку ответа на CONNECT: "HTTP/1.x 200 ...".
fn connect_response_ok(resp: &[u8]) -> bool {
    let text = String::from_utf8_lossy(resp);
    let first_line = text.lines().next().unwrap_or("");
    if !first_line.starts_with("HTTP/1.") {
        return false;
    }
    first_line
        .split_whitespace()
        .nth(1)
        .map(|code| code == "200")
        .unwrap_or(false)
}

/// Убирает Connection/Proxy-Connection заголовки и ставит `Connection: close`.
/// Нужно для plain-HTTP: после первого запроса соединение уходит в raw relay,
/// и последующие keep-alive запросы прошли бы мимо переписывания заголовков.
fn force_connection_close(headers: &str) -> String {
    let mut lines: Vec<&str> = headers.split("\r\n").collect();
    lines.retain(|l| {
        let lower = l.to_ascii_lowercase();
        !(lower.starts_with("connection:") || lower.starts_with("proxy-connection:"))
    });
    let mut out = lines.join("\r\n");
    out.push_str("\r\nConnection: close");
    out
}

/// Вставляет строку после первой строки HTTP-запроса.
fn inject_after_first_line(headers: &str, line: &str) -> String {
    if let Some(pos) = headers.find("\r\n") {
        let (first, rest) = headers.split_at(pos + 2);
        format!("{}{}\r\n{}", first, line, rest)
    } else {
        format!("{}\r\n{}", headers, line)
    }
}

/// Парсит "CONNECT host:port HTTP/1.1" → (host, port).
fn parse_connect_target(headers: &str) -> Option<(String, u16)> {
    let first_line = headers.lines().next()?;
    let mut parts = first_line.splitn(3, ' ');
    if parts.next()? != "CONNECT" {
        return None;
    }
    // CONNECT names the port always (RFC 9110 9.3.6).
    split_authority(parts.next()?, None)
}

/// `host:port`, `[v6]:port`, or without the port when `default` is given →
/// (host without the brackets, port). The colons of an IPv6 address are not
/// the port's: only the one after `]` is. A port that is not a number is
/// not taken for the default.
fn split_authority(authority: &str, default: Option<u16>) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        match after {
            "" => (host, None),
            _ => (host, Some(after.strip_prefix(':')?)),
        }
    } else {
        match authority.rsplit_once(':') {
            // A bare IPv6 address has no port to split off.
            Some((host, _)) if host.contains(':') => (authority, None),
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let port = match port {
        Some(p) => p.parse().ok()?,
        None => default?,
    };
    Some((host.to_string(), port))
}

/// Парсит первую строку HTTP-запроса → (host, port).
/// Работает для абсолютных URL: "GET http://example.com/path HTTP/1.1"
fn parse_http_target(headers: &str) -> Option<(String, u16)> {
    let first_line = headers.lines().next()?;
    let mut parts = first_line.splitn(3, ' ');
    parts.next()?; // method
    let url = parts.next()?;

    let without_scheme = if let Some(rest) = url.strip_prefix("https://") {
        (rest, 443u16)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (rest, 80u16)
    } else {
        // Если URL относительный — попробуем Host header
        let host = headers
            .lines()
            .find(|l| l.to_lowercase().starts_with("host:"))?
            .split_once(':')?
            .1
            .trim();
        return split_authority(host, Some(80));
    };

    let host_part = match without_scheme.0.find('/') {
        Some(s) => &without_scheme.0[..s],
        None => without_scheme.0,
    };
    split_authority(host_part, Some(without_scheme.1))
}

/// Переписывает абсолютный URL в первой строке запроса на relative path.
fn rewrite_to_relative(headers: &str) -> String {
    let first_line = match headers.lines().next() {
        Some(l) => l,
        None => return headers.to_string(),
    };
    let mut parts = first_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET");
    let url = parts.next().unwrap_or("/");
    let version = parts.next().unwrap_or("HTTP/1.1");

    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let path = if let Some(without_scheme) = without_scheme {
        if let Some(s) = without_scheme.find('/') {
            &without_scheme[s..]
        } else {
            "/"
        }
    } else {
        url
    };

    let new_first = format!("{} {} {}", method, path, version);
    let rest = &headers[first_line.len()..];
    format!("{}{}", new_first, rest)
}

// ── SSH upstream ──────────────────────────────────────────────────────────────

async fn handle_ssh_upstream(
    mut client: TcpStream,
    session: &crate::proxy::ssh::SharedSession,
) -> anyhow::Result<()> {
    let (headers_str, body) = read_http_headers(&mut client).await?;

    let is_connect = headers_str.starts_with("CONNECT ");

    let (target_host, target_port) = if is_connect {
        parse_connect_target(&headers_str).ok_or_else(|| anyhow::anyhow!("bad CONNECT line"))?
    } else {
        parse_http_target(&headers_str).ok_or_else(|| anyhow::anyhow!("bad HTTP request line"))?
    };

    let channel = session.open_channel(&target_host, target_port).await?;
    let stream = channel.into_stream();
    let (mut ssh_r, mut ssh_w) = tokio::io::split(stream);

    if is_connect {
        client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await?;
        // Клиент мог прислать байты (например, TLS ClientHello) сразу за CONNECT
        if !body.is_empty() {
            ssh_w.write_all(&body).await?;
        }
    } else {
        // Connection: close — см. комментарий в handle_http_upstream.
        let relative = force_connection_close(&rewrite_to_relative(&headers_str));
        ssh_w.write_all(relative.as_bytes()).await?;
        ssh_w.write_all(b"\r\n\r\n").await?;
        if !body.is_empty() {
            ssh_w.write_all(&body).await?;
        }
    }

    // SSH-канал — не TcpStream, поэтому вместо copy_bidirectional гоняем оба
    // направления параллельно и корректно закрываем каждую сторону на EOF,
    // не обрывая встречное направление (half-close).
    let (mut cr, mut cw) = client.into_split();
    let client_to_ssh = async {
        let _ = tokio::io::copy(&mut cr, &mut ssh_w).await;
        let _ = ssh_w.shutdown().await;
    };
    let ssh_to_client = async {
        let _ = tokio::io::copy(&mut ssh_r, &mut cw).await;
        let _ = cw.shutdown().await;
    };
    tokio::join!(client_to_ssh, ssh_to_client);

    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_connect_target_basic() {
        assert_eq!(
            parse_connect_target("CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n"),
            Some(("example.com".to_string(), 443))
        );
    }

    #[test]
    fn parse_connect_target_rejects_non_connect() {
        assert!(parse_connect_target("GET / HTTP/1.1").is_none());
        assert!(parse_connect_target("CONNECT example.com HTTP/1.1").is_none());
    }

    #[test]
    fn parse_http_target_absolute_http_and_https() {
        assert_eq!(
            parse_http_target("GET http://x.com/path HTTP/1.1\r\n\r\n"),
            Some(("x.com".to_string(), 80))
        );
        assert_eq!(
            parse_http_target("GET https://x.com/path HTTP/1.1\r\n\r\n"),
            Some(("x.com".to_string(), 443))
        );
    }

    #[test]
    fn parse_http_target_explicit_port() {
        assert_eq!(
            parse_http_target("GET http://x.com:8080/ HTTP/1.1\r\n\r\n"),
            Some(("x.com".to_string(), 8080))
        );
    }

    #[test]
    fn parse_http_target_relative_falls_back_to_host_header() {
        assert_eq!(
            parse_http_target("GET /path HTTP/1.1\r\nHost: fallback.com:1234\r\n\r\n"),
            Some(("fallback.com".to_string(), 1234))
        );
    }

    #[test]
    fn rewrite_to_relative_strips_scheme_and_authority() {
        let out = rewrite_to_relative("GET http://x.com/a/b?q=1 HTTP/1.1\r\nHost: x.com\r\n\r\n");
        assert!(out.starts_with("GET /a/b?q=1 HTTP/1.1"));
        // Preserves the remaining headers verbatim
        assert!(out.contains("Host: x.com"));
    }

    #[test]
    fn rewrite_to_relative_leaves_relative_urls_untouched() {
        let out = rewrite_to_relative("GET /already/relative HTTP/1.1\r\n\r\n");
        assert!(out.starts_with("GET /already/relative HTTP/1.1"));
    }

    #[test]
    fn inject_after_first_line_inserts_between_request_line_and_headers() {
        let out = inject_after_first_line("GET / HTTP/1.1\r\nHost: x\r\n\r\n", "X-Added: 1");
        assert_eq!(out, "GET / HTTP/1.1\r\nX-Added: 1\r\nHost: x\r\n\r\n");
    }

    #[test]
    fn find_header_end_locates_blank_line() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(14));
        assert!(find_header_end(b"incomplete\r\n").is_none());
    }

    #[test]
    fn ipv6_targets_keep_their_colons() {
        assert_eq!(
            parse_connect_target("CONNECT [::1]:443 HTTP/1.1\r\n\r\n"),
            Some(("::1".to_string(), 443))
        );
        assert_eq!(
            parse_connect_target("CONNECT [2001:db8::1]:8443 HTTP/1.1\r\n\r\n"),
            Some(("2001:db8::1".to_string(), 8443))
        );
        // CONNECT without a port is no CONNECT.
        assert!(parse_connect_target("CONNECT [::1] HTTP/1.1\r\n\r\n").is_none());
        assert_eq!(
            parse_http_target("GET http://[::1]/x HTTP/1.1\r\n\r\n"),
            Some(("::1".to_string(), 80))
        );
        assert_eq!(
            parse_http_target("GET https://[2001:db8::1]:8080/x HTTP/1.1\r\n\r\n"),
            Some(("2001:db8::1".to_string(), 8080))
        );
        assert_eq!(
            parse_http_target("GET /x HTTP/1.1\r\nHost: [::1]\r\n\r\n"),
            Some(("::1".to_string(), 80))
        );
        assert_eq!(
            parse_http_target("GET /x HTTP/1.1\r\nHost: [::1]:8080\r\n\r\n"),
            Some(("::1".to_string(), 8080))
        );
        // Without brackets an IPv6 address has no port to cut off.
        assert_eq!(split_authority("::1", Some(80)), Some(("::1".to_string(), 80)));
        // A port that is not one is refused, not replaced by the default.
        assert!(parse_http_target("GET http://x.com:abc/ HTTP/1.1\r\n\r\n").is_none());
        assert!(split_authority("[::1", Some(80)).is_none());
        assert!(split_authority("[::1]x", Some(80)).is_none());
        assert!(split_authority(":80", None).is_none());
    }

    /// A SOCKS5 server that reads the greeting, answers `method` and stops.
    async fn socks_server(method: u8) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut head = [0u8; 2];
            s.read_exact(&mut head).await.unwrap();
            let mut methods = vec![0u8; head[1] as usize];
            s.read_exact(&mut methods).await.unwrap();
            s.write_all(&[0x05, method]).await.unwrap();
            [head.to_vec(), methods].concat()
        });
        (port, task)
    }

    #[tokio::test]
    async fn socks5_with_credentials_offers_only_user_pass_and_refuses_no_auth() {
        let (port, server) = socks_server(0x00).await;
        let err = socks5_connect("127.0.0.1", port, "x.com", 443, Some("u"), Some("p"))
            .await
            .expect_err("no auth must be refused when credentials are set");
        assert!(err.to_string().contains("skipped the authentication"), "{err}");
        assert_eq!(server.await.unwrap(), vec![0x05, 0x01, 0x02]);
    }

    #[tokio::test]
    async fn socks5_without_credentials_offers_no_auth() {
        let (port, server) = socks_server(0xFF).await;
        assert!(socks5_connect("127.0.0.1", port, "x.com", 443, None, None)
            .await
            .is_err());
        assert_eq!(server.await.unwrap(), vec![0x05, 0x01, 0x00]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_client_row_of_proc_net_tcp_gives_its_uid() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
            \x20  0: 0100007F:9C40 0100007F:D431 01 00000000:00000000 00:00000000 00000000  1000        0 11 1 0 20 4 30 10 -1\n\
            \x20  1: 0100007F:D431 0100007F:9C40 01 00000000:00000000 00:00000000 00000000  1001        0 12 1 0 20 4 30 10 -1\n";
        let relay: std::net::SocketAddrV4 = "127.0.0.1:40000".parse().unwrap();
        let client: std::net::SocketAddrV4 = "127.0.0.1:54321".parse().unwrap();
        if cfg!(target_endian = "little") {
            assert_eq!(tcp_peer_uid(table, relay, client), Some(1001));
            assert_eq!(tcp_peer_uid(table, client, relay), Some(1000));
        }
        let other: std::net::SocketAddrV4 = "127.0.0.1:1".parse().unwrap();
        assert_eq!(tcp_peer_uid(table, relay, other), None);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_connection_of_this_user_is_let_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let _client = TcpStream::connect(local).await.unwrap();
        let (_server, peer) = listener.accept().await.unwrap();
        let table = std::fs::read_to_string("/proc/net/tcp").unwrap();
        let (std::net::SocketAddr::V4(l), std::net::SocketAddr::V4(p)) = (local, peer) else {
            panic!("loopback v4")
        };
        use std::os::unix::fs::MetadataExt;
        let me = std::fs::metadata("/proc/self").unwrap().uid();
        assert_eq!(tcp_peer_uid(&table, l, p), Some(me));
        assert!(!peer_of_another_user(local, peer).await);
    }
}

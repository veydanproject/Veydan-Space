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
}

/// Запускает локальный HTTP-прокси на 127.0.0.1:<random port>.
/// Принимает CONNECT и plain HTTP запросы от Firefox,
/// пробрасывает через HTTP или SOCKS5 upstream.
pub async fn spawn(upstream: Upstream) -> anyhow::Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_port = listener.local_addr()?.port();

    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

    let upstream = std::sync::Arc::new(upstream);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((client, _)) => {
                            let up = std::sync::Arc::clone(&upstream);
                            tokio::spawn(async move { let _ = handle(client, up).await; });
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

    // Предлагаем методы аутентификации
    if has_auth {
        s.write_all(&[0x05, 0x02, 0x00, 0x02]).await?; // no-auth + user/pass
    } else {
        s.write_all(&[0x05, 0x01, 0x00]).await?; // no-auth only
    }

    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await?;
    anyhow::ensure!(resp[0] == 0x05, "SOCKS5: bad version");

    match resp[1] {
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

    // CONNECT через DOMAINNAME (ATYP=0x03)
    let mut req = vec![0x05, 0x01, 0x00, 0x03];
    req.push(target_host.len() as u8);
    req.extend_from_slice(target_host.as_bytes());
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
    let addr = parts.next()?;
    let colon = addr.rfind(':')?;
    let host = addr[..colon].to_string();
    let port: u16 = addr[colon + 1..].parse().ok()?;
    Some((host, port))
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
        let (h, p) = if let Some(c) = host.rfind(':') {
            (&host[..c], host[c + 1..].parse().unwrap_or(80))
        } else {
            (host, 80)
        };
        return Some((h.to_string(), p));
    };

    let (host_part, _path) = if let Some(s) = without_scheme.0.find('/') {
        (&without_scheme.0[..s], &without_scheme.0[s..])
    } else {
        (without_scheme.0, "/")
    };

    let (host, port) = if let Some(c) = host_part.rfind(':') {
        (
            &host_part[..c],
            host_part[c + 1..].parse().unwrap_or(without_scheme.1),
        )
    } else {
        (host_part, without_scheme.1)
    };

    Some((host.to_string(), port))
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
}

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct CallbackError {
    code: &'static str,
    message: &'static str,
}
impl CallbackError {
    fn remote(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn message(&self) -> &'static str {
        self.message
    }
}
use crate::dropbox_auth::{AuthorizationRequest, CALLBACK_URI};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};
use url::Url;
use zeroize::Zeroizing;

fn failure(code: &'static str, message: &'static str) -> CallbackError {
    CallbackError::remote(code, message)
}
fn invalid_callback() -> CallbackError {
    failure(
        "dropbox_callback_invalid",
        "Dropbox authorization returned an invalid callback. Run setup again.",
    )
}

pub fn authorize(
    request: &AuthorizationRequest,
    on_link: impl FnOnce(&Url) -> Result<(), ()>,
    cancelled: &AtomicBool,
) -> Result<Zeroizing<String>, CallbackError> {
    let callback = Url::parse(CALLBACK_URI).map_err(|_| invalid_callback())?;
    let address: SocketAddr = format!(
        "127.0.0.1:{}",
        callback.port().ok_or_else(invalid_callback)?
    )
    .parse()
    .map_err(|_| invalid_callback())?;
    let listener = bind(address).map_err(|_| failure("dropbox_callback_unavailable",
        "Mallard could not bind the Dropbox callback port. Close the other setup attempt and try again."))?;
    receive_cancellable(
        listener,
        request,
        on_link,
        Duration::from_secs(180),
        cancelled,
    )
}

fn bind(address: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    #[cfg(windows)]
    socket.set_exclusive_address_use(true)?;
    socket.bind(&address.into())?;
    socket.listen(16)?;
    Ok(socket.into())
}

fn receive_cancellable(
    listener: TcpListener,
    request: &AuthorizationRequest,
    opener: impl FnOnce(&Url) -> Result<(), ()>,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<Zeroizing<String>, CallbackError> {
    let address = listener.local_addr().map_err(|_| invalid_callback())?;
    if !address.ip().is_loopback() {
        return Err(invalid_callback());
    }
    listener
        .set_nonblocking(true)
        .map_err(|_| invalid_callback())?;
    opener(&request.url).map_err(|_| {
        failure(
            "dropbox_browser_failed",
            "Mallard could not open the Dropbox authorization page.",
        )
    })?;
    let deadline = Instant::now() + timeout;
    let mut attempts = 0;
    while Instant::now() < deadline && attempts < 32 {
        if cancelled.load(Ordering::Acquire) {
            return Err(failure(
                "dropbox_setup_cancelled",
                "Dropbox setup was cancelled.",
            ));
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                attempts += 1;
                if !peer.ip().is_loopback() {
                    continue;
                }
                let result = read_callback(&mut stream, &address.to_string(), &request.state);
                let message = if result.is_ok() {
                    "Dropbox authorization received. Return to Mallard and wait for setup to finish."
                } else {
                    "Dropbox authorization was not completed. Return to Mallard."
                };
                // Replace the callback URL in browser history. No code is
                // echoed in the page, and no external resource is loaded.
                let nonce = String::from(
                    mallard_domain::HostId::generate().map_err(|_| invalid_callback())?,
                );
                let body = format!("<!doctype html><title>Dropbox authorization</title><p>{message}</p><script nonce=\"{nonce}\">history.replaceState(null, '', '/dropbox/complete');</script>");
                let status = if result.is_ok() {
                    "200 OK"
                } else {
                    "400 Bad Request"
                };
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'nonce-{nonce}'; frame-ancestors 'none'; base-uri 'none'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                // An unrelated request cannot consume the pending OAuth flow.
                match result {
                    Ok(code) => return Ok(code),
                    Err(error) if error.code() == "dropbox_consent_denied" => return Err(error),
                    Err(_) => {}
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(_) => return Err(invalid_callback()),
        }
    }
    Err(failure(
        "dropbox_authorization_timeout",
        "Dropbox authorization timed out. Run setup again when you are ready.",
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub fn receive(
    listener: TcpListener,
    request: &AuthorizationRequest,
    opener: impl FnOnce(&Url) -> Result<(), ()>,
    timeout: Duration,
) -> Result<Zeroizing<String>, CallbackError> {
    receive_cancellable(listener, request, opener, timeout, &AtomicBool::new(false))
}

fn read_callback(
    stream: &mut TcpStream,
    expected_host: &str,
    state: &str,
) -> Result<Zeroizing<String>, CallbackError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| invalid_callback())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| invalid_callback())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Zeroizing::new(Vec::new());
    loop {
        if bytes.len() >= 16 * 1024 || Instant::now() >= deadline {
            return Err(invalid_callback());
        }
        let mut chunk = Zeroizing::new([0u8; 1024]);
        let read = stream
            .read(chunk.as_mut())
            .map_err(|_| invalid_callback())?;
        if read == 0 {
            return Err(invalid_callback());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    parse_callback(&bytes, expected_host, state)
}

fn parse_callback(
    bytes: &[u8],
    expected_host: &str,
    state: &str,
) -> Result<Zeroizing<String>, CallbackError> {
    if bytes.len() > 16 * 1024 {
        return Err(invalid_callback());
    }
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    let length = match request.parse(bytes).map_err(|_| invalid_callback())? {
        httparse::Status::Complete(length) => length,
        httparse::Status::Partial => return Err(invalid_callback()),
    };
    if length != bytes.len() || request.method != Some("GET") {
        return Err(invalid_callback());
    }
    let hosts = request
        .headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("host"))
        .collect::<Vec<_>>();
    if hosts.len() != 1
        || hosts[0].value != expected_host.as_bytes()
        || request.headers.iter().any(|h| {
            h.name.eq_ignore_ascii_case("transfer-encoding")
                || h.name.eq_ignore_ascii_case("content-length")
        })
    {
        return Err(invalid_callback());
    }
    let path = request.path.ok_or_else(invalid_callback)?;
    if !path.starts_with("/dropbox/callback?") || path.contains('#') {
        return Err(invalid_callback());
    }
    let mut values = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(
        path.split_once('?')
            .ok_or_else(invalid_callback)?
            .1
            .as_bytes(),
    ) {
        if !["state", "code", "error", "error_description"].contains(&key.as_ref())
            || values
                .insert(key.to_string(), Zeroizing::new(value.to_string()))
                .is_some()
        {
            return Err(invalid_callback());
        }
    }
    if values.get("state").map(|s| s.as_str()) != Some(state) {
        return Err(invalid_callback());
    }
    if values.contains_key("error") && !values.contains_key("code") {
        return Err(failure(
            "dropbox_consent_denied",
            "Dropbox access was not approved. Setup stopped.",
        ));
    }
    if values.contains_key("error") || values.contains_key("error_description") {
        return Err(invalid_callback());
    }
    let code = values.remove("code").ok_or_else(invalid_callback)?;
    if code.is_empty() || code.len() > 4096 || !code.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid_callback());
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_rejects_wrong_state_host_duplicates_bodies_and_ambiguous_results() {
        let valid = b"GET /dropbox/callback?code=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n";
        assert_eq!(
            parse_callback(valid, "127.0.0.1:53682", "expected")
                .unwrap()
                .as_str(),
            "private-code"
        );
        for request in [
            "GET /dropbox/callback?code=private-code&state=wrong HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
            "GET /dropbox/callback?code=private-code&state=expected HTTP/1.1\r\nHost: attacker.test\r\n\r\n",
            "GET /dropbox/callback?code=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\nHost: 127.0.0.1:53682\r\n\r\n",
            "GET /dropbox/callback?code=private-code&code=second&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
            "GET /dropbox/callback?code=private-code&state=expected&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
            "GET /dropbox/callback?code=private-code&state=expected&error=denied HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
            "POST /dropbox/callback?code=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
            "GET /dropbox/callback?code=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\nContent-Length: 1\r\n\r\nx",
            "GET /wrong?code=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
        ] {
            let error = parse_callback(request.as_bytes(), "127.0.0.1:53682", "expected").unwrap_err();
            assert!(!error.message().contains("private-code"));
        }
        let denied = b"GET /dropbox/callback?error=access_denied&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n";
        assert_eq!(
            parse_callback(denied, "127.0.0.1:53682", "expected")
                .unwrap_err()
                .code(),
            "dropbox_consent_denied"
        );
    }

    #[test]
    fn browser_callback_finishes_without_terminal_code_entry_or_code_in_response() {
        let listener = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_addr().unwrap();
        let request = AuthorizationRequest::new(crate::dropbox_auth::SETUP_SCOPES).unwrap();
        let state = request.state.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let code = receive(listener, &request, |_| {
            std::thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                write!(stream, "GET /dropbox/callback?code=private-code&state={state} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                tx.send(response).unwrap();
            });
            Ok(())
        }, Duration::from_secs(5)).unwrap();
        assert_eq!(code.as_str(), "private-code");
        let response = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!response.contains("private-code"));
        assert!(!response.contains(request.state.as_str()));
        assert!(
            response.contains("history.replaceState")
                && response.contains("Cache-Control: no-store")
        );
    }

    #[test]
    fn callback_bind_conflict_and_timeout_stop_cleanly() {
        let listener = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(bind(listener.local_addr().unwrap()).is_err());
        let request = AuthorizationRequest::new(crate::dropbox_auth::SETUP_SCOPES).unwrap();
        assert_eq!(
            receive(listener, &request, |_| Ok(()), Duration::from_millis(1))
                .unwrap_err()
                .code(),
            "dropbox_authorization_timeout"
        );
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancel_releases_the_listener_before_a_retry() {
        let request = AuthorizationRequest::new(crate::dropbox_auth::SETUP_SCOPES).unwrap();
        let listener = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_addr().unwrap();
        let cancelled = AtomicBool::new(false);
        let result = receive_cancellable(
            listener,
            &request,
            |_| {
                cancelled.store(true, Ordering::Release);
                Ok(())
            },
            Duration::from_secs(5),
            &cancelled,
        );
        assert_eq!(result.unwrap_err().code(), "dropbox_setup_cancelled");
        assert!(bind(address).is_ok());
    }
}

//! The loopback receiver every browser sign-in shares. A sign-in binds a
//! port on 127.0.0.1, hands the authorization link to the caller (never
//! opening it itself), and waits for the provider to redirect the browser
//! back with the result under the declared query key. The result never
//! appears in the page the browser receives, and an unrelated request
//! cannot consume the pending flow.
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use url::Url;
use zeroize::Zeroizing;

/// One product's callback shape: the path the provider redirects to, the
/// query key that carries the result, and the fixed codes and sentences its
/// failures are reported with.
pub struct LoopbackCallback {
    /// The callback path, e.g. `/dropbox/callback`.
    pub path: &'static str,
    /// The query key carrying the authorization result (`code`, `grant`).
    pub result_key: &'static str,
    /// Where browser history is rewritten to once the callback was consumed.
    pub completion_path: &'static str,
    /// The page title the browser shows.
    pub title: &'static str,
    /// Printed in the page once the result was received.
    pub received_message: &'static str,
    /// Printed in the page when the callback was not a valid result.
    pub failed_message: &'static str,
    pub errors: LoopbackErrors,
}

/// The fixed `(code, message)` pairs one product reports.
pub struct LoopbackErrors {
    pub invalid: (&'static str, &'static str),
    pub unavailable: (&'static str, &'static str),
    pub browser_failed: (&'static str, &'static str),
    pub cancelled: (&'static str, &'static str),
    pub denied: (&'static str, &'static str),
    pub timeout: (&'static str, &'static str),
}

#[derive(Debug)]
pub struct CallbackError {
    code: &'static str,
    message: &'static str,
}
impl CallbackError {
    fn from_pair((code, message): (&'static str, &'static str)) -> Self {
        Self { code, message }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn message(&self) -> &'static str {
        self.message
    }
}

/// A bound loopback port waiting for one callback.
#[derive(Debug)]
pub struct LoopbackListener {
    listener: TcpListener,
    address: SocketAddr,
}

impl LoopbackListener {
    /// Bind `port` on 127.0.0.1; `0` takes any free port, which `port()`
    /// then reports so the sign-in can be started with it.
    pub fn bind(callback: &LoopbackCallback, port: u16) -> Result<Self, CallbackError> {
        let address: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .map_err(|_| CallbackError::from_pair(callback.errors.invalid))?;
        let listener =
            bind(address).map_err(|_| CallbackError::from_pair(callback.errors.unavailable))?;
        let address = listener
            .local_addr()
            .map_err(|_| CallbackError::from_pair(callback.errors.invalid))?;
        Ok(Self { listener, address })
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// Hand the link over, then wait for the callback carrying `state`.
    /// Cancellation and the timeout release the port before they return.
    pub fn receive(
        self,
        callback: &LoopbackCallback,
        url: &Url,
        state: &str,
        opener: impl FnOnce(&Url) -> Result<(), ()>,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<Zeroizing<String>, CallbackError> {
        let invalid = || CallbackError::from_pair(callback.errors.invalid);
        if !self.address.ip().is_loopback() {
            return Err(invalid());
        }
        self.listener.set_nonblocking(true).map_err(|_| invalid())?;
        opener(url).map_err(|_| CallbackError::from_pair(callback.errors.browser_failed))?;
        let deadline = Instant::now() + timeout;
        let mut attempts = 0;
        while Instant::now() < deadline && attempts < 32 {
            if cancelled.load(Ordering::Acquire) {
                return Err(CallbackError::from_pair(callback.errors.cancelled));
            }
            match self.listener.accept() {
                Ok((mut stream, peer)) => {
                    attempts += 1;
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let result =
                        read_callback(callback, &mut stream, &self.address.to_string(), state);
                    let message = if result.is_ok() {
                        callback.received_message
                    } else {
                        callback.failed_message
                    };
                    // Replace the callback URL in browser history. No result
                    // is echoed in the page, and no external resource is
                    // loaded.
                    let nonce =
                        String::from(mallard_domain::HostId::generate().map_err(|_| invalid())?);
                    let body = format!(
                        "<!doctype html><title>{}</title><p>{message}</p><script nonce=\"{nonce}\">history.replaceState(null, '', '{}');</script>",
                        callback.title, callback.completion_path
                    );
                    let status = if result.is_ok() {
                        "200 OK"
                    } else {
                        "400 Bad Request"
                    };
                    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'nonce-{nonce}'; frame-ancestors 'none'; base-uri 'none'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                    // An unrelated request cannot consume the pending flow.
                    match result {
                        Ok(value) => return Ok(value),
                        Err(error) if error.code() == callback.errors.denied.0 => {
                            return Err(error)
                        }
                        Err(_) => {}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Err(_) => return Err(invalid()),
            }
        }
        Err(CallbackError::from_pair(callback.errors.timeout))
    }
}

/// Bind the product's fixed port and wait up to three minutes for the
/// callback of `url`.
pub fn authorize(
    callback: &LoopbackCallback,
    url: &Url,
    state: &str,
    port: u16,
    on_link: impl FnOnce(&Url) -> Result<(), ()>,
    cancelled: &AtomicBool,
) -> Result<Zeroizing<String>, CallbackError> {
    LoopbackListener::bind(callback, port)?.receive(
        callback,
        url,
        state,
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
    set_exclusive_address_use(&socket)?;
    socket.bind(&address.into())?;
    socket.listen(16)?;
    Ok(socket.into())
}

/// `SO_EXCLUSIVEADDRUSE` keeps another process from binding the callback
/// port underneath us, even one that sets `SO_REUSEADDR`. `socket2` has no
/// setter for it, so the option is applied directly.
#[cfg(windows)]
fn set_exclusive_address_use(socket: &Socket) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, SOCKET_ERROR, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
    };

    let enable: i32 = 1;
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            (&enable as *const i32).cast::<u8>(),
            std::mem::size_of::<i32>() as i32,
        )
    };
    if result == SOCKET_ERROR {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn read_callback(
    callback: &LoopbackCallback,
    stream: &mut TcpStream,
    expected_host: &str,
    state: &str,
) -> Result<Zeroizing<String>, CallbackError> {
    let invalid = || CallbackError::from_pair(callback.errors.invalid);
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| invalid())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| invalid())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Zeroizing::new(Vec::new());
    loop {
        if bytes.len() >= 16 * 1024 || Instant::now() >= deadline {
            return Err(invalid());
        }
        let mut chunk = Zeroizing::new([0u8; 1024]);
        let read = stream.read(chunk.as_mut()).map_err(|_| invalid())?;
        if read == 0 {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    parse_callback(callback, &bytes, expected_host, state)
}

/// Parse one raw HTTP request: a GET of the callback path from the
/// expected host, carrying exactly the state, and either the result or a
/// provider error. Anything else is invalid.
pub fn parse_callback(
    callback: &LoopbackCallback,
    bytes: &[u8],
    expected_host: &str,
    state: &str,
) -> Result<Zeroizing<String>, CallbackError> {
    let invalid = || CallbackError::from_pair(callback.errors.invalid);
    if bytes.len() > 16 * 1024 {
        return Err(invalid());
    }
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    let length = match request.parse(bytes).map_err(|_| invalid())? {
        httparse::Status::Complete(length) => length,
        httparse::Status::Partial => return Err(invalid()),
    };
    if length != bytes.len() || request.method != Some("GET") {
        return Err(invalid());
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
        return Err(invalid());
    }
    let path = request.path.ok_or_else(invalid)?;
    let Some((route, query)) = path.split_once('?') else {
        return Err(invalid());
    };
    if route != callback.path || path.contains('#') {
        return Err(invalid());
    }
    let mut values = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if !["state", callback.result_key, "error", "error_description"].contains(&key.as_ref())
            || values
                .insert(key.to_string(), Zeroizing::new(value.to_string()))
                .is_some()
        {
            return Err(invalid());
        }
    }
    if values.get("state").map(|s| s.as_str()) != Some(state) {
        return Err(invalid());
    }
    if values.contains_key("error") && !values.contains_key(callback.result_key) {
        return Err(CallbackError::from_pair(callback.errors.denied));
    }
    if values.contains_key("error") || values.contains_key("error_description") {
        return Err(invalid());
    }
    let result = values.remove(callback.result_key).ok_or_else(invalid)?;
    if result.is_empty() || result.len() > 4096 || !result.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dropbox_auth::{AuthorizationRequest, DROPBOX_CALLBACK, SETUP_SCOPES};

    #[test]
    fn callback_rejects_wrong_state_host_duplicates_bodies_and_ambiguous_results() {
        let valid = b"GET /dropbox/callback?code=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n";
        assert_eq!(
            parse_callback(&DROPBOX_CALLBACK, valid, "127.0.0.1:53682", "expected")
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
            "GET /mallard/callback?grant=private-code&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n",
        ] {
            let error = parse_callback(&DROPBOX_CALLBACK, request.as_bytes(), "127.0.0.1:53682", "expected").unwrap_err();
            assert!(!error.message().contains("private-code"));
        }
        let denied = b"GET /dropbox/callback?error=access_denied&state=expected HTTP/1.1\r\nHost: 127.0.0.1:53682\r\n\r\n";
        assert_eq!(
            parse_callback(&DROPBOX_CALLBACK, denied, "127.0.0.1:53682", "expected")
                .unwrap_err()
                .code(),
            "dropbox_consent_denied"
        );
    }

    #[test]
    fn browser_callback_finishes_without_terminal_code_entry_or_code_in_response() {
        let listener = LoopbackListener::bind(&DROPBOX_CALLBACK, 0).unwrap();
        let address = listener.address;
        let request = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
        let state = request.state.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let code = listener
            .receive(
                &DROPBOX_CALLBACK,
                &request.url,
                &request.state,
                |_| {
                    std::thread::spawn(move || {
                        let mut stream = TcpStream::connect(address).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        write!(stream, "GET /dropbox/callback?code=private-code&state={state} HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
                        let mut response = String::new();
                        stream.read_to_string(&mut response).unwrap();
                        tx.send(response).unwrap();
                    });
                    Ok(())
                },
                Duration::from_secs(5),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(code.as_str(), "private-code");
        let response = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!response.contains("private-code"));
        assert!(!response.contains(request.state.as_str()));
        assert!(
            response.contains("history.replaceState")
                && response.contains("Cache-Control: no-store")
                && response.contains("/dropbox/complete")
        );
    }

    #[test]
    fn callback_bind_conflict_and_timeout_stop_cleanly() {
        let listener = LoopbackListener::bind(&DROPBOX_CALLBACK, 0).unwrap();
        assert_eq!(
            LoopbackListener::bind(&DROPBOX_CALLBACK, listener.port())
                .unwrap_err()
                .code(),
            "dropbox_callback_unavailable"
        );
        let request = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
        assert_eq!(
            listener
                .receive(
                    &DROPBOX_CALLBACK,
                    &request.url,
                    &request.state,
                    |_| Ok(()),
                    Duration::from_millis(1),
                    &AtomicBool::new(false),
                )
                .unwrap_err()
                .code(),
            "dropbox_authorization_timeout"
        );
    }

    #[test]
    fn cancel_releases_the_listener_before_a_retry() {
        let request = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
        let listener = LoopbackListener::bind(&DROPBOX_CALLBACK, 0).unwrap();
        let port = listener.port();
        let cancelled = AtomicBool::new(false);
        let result = listener.receive(
            &DROPBOX_CALLBACK,
            &request.url,
            &request.state,
            |_| {
                cancelled.store(true, Ordering::Release);
                Ok(())
            },
            Duration::from_secs(5),
            &cancelled,
        );
        assert_eq!(result.unwrap_err().code(), "dropbox_setup_cancelled");
        assert!(LoopbackListener::bind(&DROPBOX_CALLBACK, port).is_ok());
    }
}

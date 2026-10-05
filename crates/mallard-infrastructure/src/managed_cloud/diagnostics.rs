//! Fixed transport classifications. Never format an error or its sources:
//! they can contain request URLs, credentials, or local paths.
use std::error::Error;
use std::io;
use std::sync::OnceLock;
use tokio::sync::broadcast;

fn events() -> &'static broadcast::Sender<String> {
    static EVENTS: OnceLock<broadcast::Sender<String>> = OnceLock::new();
    EVENTS.get_or_init(|| broadcast::channel(64).0)
}

/// The desktop persists these bounded, sanitized upload facts locally.
pub fn upload_diagnostics() -> broadcast::Receiver<String> {
    events().subscribe()
}

pub(super) fn report_upload(message: String) {
    let _ = events().send(message);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TransportCause {
    pub summary: Option<&'static str>,
    pub tags: Vec<&'static str>,
    pub os_code: Option<i32>,
}

impl TransportCause {
    pub fn inspect(error: &(dyn Error + 'static)) -> Self {
        let mut result = Self::default();
        let mut source = Some(error);
        // Also bound malformed or cyclic source chains.
        for _ in 0..12 {
            let Some(error) = source else { break };
            let classified = if let Some(error) = error.downcast_ref::<io::Error>() {
                result.os_code = error.raw_os_error().or(result.os_code);
                Some(match error.kind() {
                    io::ErrorKind::ConnectionReset => ("connection_reset", "connection reset"),
                    io::ErrorKind::ConnectionAborted => {
                        ("connection_aborted", "connection aborted")
                    }
                    io::ErrorKind::ConnectionRefused => {
                        ("connection_refused", "connection refused")
                    }
                    io::ErrorKind::BrokenPipe => ("broken_pipe", "connection closed"),
                    io::ErrorKind::UnexpectedEof => ("unexpected_eof", "connection closed"),
                    io::ErrorKind::NotConnected => ("not_connected", "connection closed"),
                    io::ErrorKind::TimedOut => ("io_timeout", "request timed out"),
                    _ => ("io_other", "network I/O failed"),
                })
            } else if let Some(error) = error.downcast_ref::<hyper::Error>() {
                if error.is_incomplete_message() {
                    Some(("http_incomplete_message", "connection closed"))
                } else if error.is_closed() {
                    Some(("http_channel_closed", "request channel closed"))
                } else if error.is_canceled() {
                    Some(("http_canceled", "request canceled"))
                } else if error.is_timeout() {
                    Some(("http_timeout", "request timed out"))
                } else if error.is_parse() {
                    Some(("http_parse", "invalid HTTP response"))
                } else if error.is_body_write_aborted() {
                    Some(("http_body_write_aborted", "upload interrupted"))
                } else if error.is_shutdown() {
                    Some(("http_shutdown", "connection closed"))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((tag, summary)) = classified {
                if !result.tags.contains(&tag) {
                    result.tags.push(tag);
                }
                result.summary = Some(summary);
            }
            source = error.source();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_causes_use_fixed_tags_and_never_error_text() {
        for (kind, expected) in [
            (io::ErrorKind::ConnectionReset, "connection_reset"),
            (io::ErrorKind::BrokenPipe, "broken_pipe"),
            (io::ErrorKind::UnexpectedEof, "unexpected_eof"),
            (io::ErrorKind::TimedOut, "io_timeout"),
            (io::ErrorKind::Other, "io_other"),
        ] {
            let error = io::Error::new(kind, "https://private/?token=secret /Users/private");
            let cause = TransportCause::inspect(&error);
            assert_eq!(cause.tags, [expected]);
            assert!(!format!("{cause:?}").contains("private"));
            assert!(!format!("{cause:?}").contains("secret"));
        }
    }

    #[test]
    fn unknown_and_cyclic_sources_are_bounded_without_formatting() {
        #[derive(Debug)]
        struct Cycle;
        impl std::fmt::Display for Cycle {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("diagnostics must not format source errors");
            }
        }
        impl Error for Cycle {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(self)
            }
        }
        assert_eq!(TransportCause::inspect(&Cycle), TransportCause::default());
    }
}

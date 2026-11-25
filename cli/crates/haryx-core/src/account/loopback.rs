//! The one-shot local listener the browser returns to after sign-in (RFC 8252 loopback redirect).
//!
//! Binds `127.0.0.1` on a port the OS picks. Supabase Auth accepts any loopback IP redirect with no
//! allow-list entry, but not `localhost` (spike, `docs/haryx-account-plan.md`), so the literal IP is
//! used. The path is `/callback/<nonce>`, random per sign-in, so nothing that merely finds the port
//! (a port-scanning page, another local process) can end the sign-in: any other request is answered
//! 404 and the wait goes on, as are requests without `code`/`error` (a `/favicon.ico`, for one).
//!
//! The redirect is answered only once the caller knows the outcome ([`Arrival::respond`]), so the
//! page never says "signed in" for a code the exchange then rejects.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::AccountError;

/// How often a non-blocking accept is retried while waiting.
const POLL: Duration = Duration::from_millis(50);
/// Total time one connection gets to send its request line and headers.
const REQUEST_BUDGET: Duration = Duration::from_secs(2);
/// Requests larger than this are not a redirect from Supabase.
const MAX_REQUEST: usize = 16 * 1024;
const NONCE_BYTES: usize = 16;

/// What the browser brought back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    /// The provider or the user refused: `error_description`, else `error`, control characters removed.
    Denied(String),
}

/// The redirect, with the browser still waiting for its page.
#[derive(Debug)]
pub struct Arrival {
    pub callback: Callback,
    stream: TcpStream,
}

impl Arrival {
    /// Answer the browser with `page`. It may already be gone; the outcome stands either way.
    pub fn respond(mut self, page: &str) {
        let _ = self.stream.write_all(response("200 OK", page).as_bytes());
    }
}

pub struct Loopback {
    listener: TcpListener,
    port: u16,
    path: String,
}

impl Loopback {
    pub fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; NONCE_BYTES]>());
        Ok(Loopback {
            listener,
            port,
            path: format!("/callback/{nonce}"),
        })
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.path)
    }

    /// Wait up to `timeout` for the redirect carrying a code or an error.
    pub fn wait(&self, timeout: Duration) -> Result<Arrival, AccountError> {
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(AccountError::Timeout(timeout));
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(arrival) = self.serve(stream) {
                        return Ok(arrival);
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
                Err(err) => return Err(AccountError::Network(err.to_string())),
            }
        }
    }

    /// Read one request. The redirect comes back unanswered; anything else is answered and dropped.
    fn serve(&self, mut stream: TcpStream) -> Option<Arrival> {
        // Accepted sockets may inherit non-blocking mode; reads here block, within the budget.
        stream.set_nonblocking(false).ok()?;
        let target = read_request_target(&mut stream, Instant::now() + REQUEST_BUDGET)?;
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        if path != self.path {
            let _ = stream.write_all(response("404 Not Found", "Not found.").as_bytes());
            return None;
        }
        match parse_callback(query) {
            Some(callback) => Some(Arrival { callback, stream }),
            None => {
                let _ = stream.write_all(response("400 Bad Request", "Missing code.").as_bytes());
                None
            }
        }
    }
}

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; font-src data:; \
         img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The request target of a `GET`, read up to the end of the headers before `deadline`.
fn read_request_target(stream: &mut TcpStream, deadline: Instant) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let left = deadline.checked_duration_since(Instant::now())?;
        stream
            .set_read_timeout(Some(left.max(Duration::from_millis(1))))
            .ok()?;
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 || buf.len() + n > MAX_REQUEST {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.lines().next()?.split(' ');
    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

fn parse_callback(query: &str) -> Option<Callback> {
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k), percent_decode(v)))
        .collect();
    let get = |name: &str| {
        pairs
            .iter()
            .find(|(k, v)| k == name && !v.is_empty())
            .map(|(_, v)| v.clone())
    };
    get("code").map(Callback::Code).or_else(|| {
        get("error").map(|error| {
            let reason = get("error_description").unwrap_or(error);
            Callback::Denied(reason.chars().filter(|c| !c.is_control()).collect())
        })
    })
}

/// Form-style decoding: `+` is a space, `%XX` a byte; malformed escapes stay as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (other, _) => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send `request` to the listener and return the response text.
    fn send(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        out
    }

    fn get(path: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
    }

    fn bound() -> (Loopback, u16, String) {
        let lb = Loopback::bind().unwrap();
        let (port, path) = (lb.port, lb.path.clone());
        (lb, port, path)
    }

    #[test]
    fn redirect_uri_uses_the_loopback_ip_and_a_secret_path() {
        let lb = Loopback::bind().unwrap();
        let other = Loopback::bind().unwrap();
        assert!(lb.port > 0);
        assert!(
            lb.redirect_uri()
                .starts_with(&format!("http://127.0.0.1:{}/callback/", lb.port))
        );
        assert_eq!(lb.path.len(), "/callback/".len() + 22);
        assert_ne!(lb.path, other.path);
    }

    #[test]
    fn stray_requests_are_answered_and_the_code_still_arrives() {
        let (lb, port, path) = bound();
        let browser = std::thread::spawn(move || {
            let favicon = send(port, &get("/favicon.ico"));
            let guessed = send(port, &get("/callback?code=injected"));
            let no_code = send(port, &get(&format!("{path}?state=x")));
            let done = send(port, &get(&format!("{path}?code=abc%2D123&extra=1")));
            (favicon, guessed, no_code, done)
        });
        let arrival = lb.wait(Duration::from_secs(10)).unwrap();
        assert_eq!(arrival.callback, Callback::Code("abc-123".into()));
        arrival.respond("<p>finished</p>");
        let (favicon, guessed, no_code, done) = browser.join().unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(
            guessed.starts_with("HTTP/1.1 404"),
            "only the secret path counts"
        );
        assert!(no_code.starts_with("HTTP/1.1 400"));
        assert!(done.starts_with("HTTP/1.1 200") && done.ends_with("<p>finished</p>"));
        assert!(done.contains("Content-Security-Policy: default-src 'none'"));
    }

    #[test]
    fn a_provider_error_is_denied_without_control_characters() {
        let (lb, port, path) = bound();
        let browser = std::thread::spawn(move || {
            send(
                port,
                &get(&format!(
                    "{path}?error=access_denied&error_description=The+user%1B%5D52+denied"
                )),
            )
        });
        let arrival = lb.wait(Duration::from_secs(10)).unwrap();
        assert_eq!(
            arrival.callback,
            Callback::Denied("The user]52 denied".into())
        );
        arrival.respond("x");
        browser.join().unwrap();
    }

    #[test]
    fn a_silent_connection_does_not_hold_up_the_sign_in() {
        let (lb, port, path) = bound();
        let started = Instant::now();
        let browser = std::thread::spawn(move || {
            // A preconnect that never sends, then the real redirect.
            let idle = TcpStream::connect(("127.0.0.1", port)).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            let done = send(port, &get(&format!("{path}?code=c")));
            drop(idle);
            done
        });
        let arrival = lb.wait(Duration::from_secs(10)).unwrap();
        arrival.respond("ok");
        browser.join().unwrap();
        assert!(started.elapsed() < REQUEST_BUDGET + Duration::from_secs(2));
    }

    #[test]
    fn a_trickling_connection_is_cut_at_its_budget() {
        let (lb, port, _path) = bound();
        let trickle = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            for _ in 0..40 {
                if stream.write_all(b"G").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        let err = lb.wait(Duration::from_millis(500)).unwrap_err();
        assert!(matches!(err, AccountError::Timeout(_)));
        assert!(started.elapsed() < REQUEST_BUDGET + Duration::from_secs(1));
        trickle.join().unwrap();
    }

    #[test]
    fn non_get_requests_are_ignored() {
        let (lb, port, path) = bound();
        let browser = std::thread::spawn(move || {
            let post = send(port, &format!("POST {path}?code=x HTTP/1.1\r\n\r\n"));
            send(port, &get(&format!("{path}?code=real")));
            post
        });
        let arrival = lb.wait(Duration::from_secs(10)).unwrap();
        assert_eq!(arrival.callback, Callback::Code("real".into()));
        arrival.respond("ok");
        browser.join().unwrap();
    }

    #[test]
    fn nothing_arriving_times_out() {
        let lb = Loopback::bind().unwrap();
        let err = lb.wait(Duration::from_millis(120)).unwrap_err();
        assert!(matches!(err, AccountError::Timeout(_)));
    }

    #[test]
    fn percent_decoding_handles_plus_escapes_and_malformed_input() {
        assert_eq!(percent_decode("a+b%20c%2Fd"), "a b c/d");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%E1%BA%A1"), "ạ");
    }
}

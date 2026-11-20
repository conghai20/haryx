//! A local HTTP stub for the OAuth/usage endpoints (`tiny_http`), so no test touches the network.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::oauth::Api;

/// One request the stub received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A canned response.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

type Route = dyn Fn(&Recorded) -> Reply + Send + Sync;

/// A stub server on an ephemeral port; stopped when dropped.
pub struct Stub {
    server: Arc<tiny_http::Server>,
    handle: Option<JoinHandle<()>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
    base: String,
}

fn respond(req: tiny_http::Request, reply: Reply) {
    let response = reply.headers.iter().fold(
        tiny_http::Response::from_string(reply.body).with_status_code(reply.status),
        |r, (k, v)| match tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            Ok(h) => r.with_header(h),
            Err(()) => r,
        },
    );
    let _ = req.respond(response);
}

impl Stub {
    pub fn start(route: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Stub {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind stub"));
        let port = server.server_addr().to_ip().expect("ip listener").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let route: Arc<Route> = Arc::new(route);
        let (srv, reqs) = (Arc::clone(&server), Arc::clone(&requests));
        let handle = std::thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let recorded = Recorded {
                    method: req.method().as_str().to_string(),
                    path: req.url().to_string(),
                    headers: req
                        .headers()
                        .iter()
                        .map(|h| {
                            (
                                h.field.as_str().as_str().to_string(),
                                h.value.as_str().to_string(),
                            )
                        })
                        .collect(),
                    body,
                };
                let reply = route(&recorded);
                reqs.lock().expect("requests lock").push(recorded);
                respond(req, reply);
            }
        });
        Stub {
            server,
            handle: Some(handle),
            requests,
            base: format!("http://127.0.0.1:{port}"),
        }
    }

    pub fn api(&self) -> Api {
        Api::with_base(&self.base)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// A base URL nothing listens on (bound, then released).
pub fn closed_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

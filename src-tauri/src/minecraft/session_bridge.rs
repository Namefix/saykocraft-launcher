use std::{
    fmt,
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::json;
use tracing::{debug, warn};

const BRIDGE_HOST: &str = "127.0.0.1";
const SESSION_TOKEN_PATH: &str = "/session-token";
const ACCESS_CODE_HEADER: &str = "x-saykocraft-code";
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

const HOST_PROPERTY: &str = "saykocraft.bridge.host";
const PORT_PROPERTY: &str = "saykocraft.bridge.port";
const CODE_PROPERTY: &str = "saykocraft.bridge.code";
const PATH_PROPERTY: &str = "saykocraft.bridge.path";

pub struct SessionBridge {
    port: u16,
    access_code: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SessionBridge {
    pub fn start(session_token: String) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let access_code = generate_access_code()?;
        let thread_access_code = access_code.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            run_bridge(listener, session_token, thread_access_code, thread_shutdown);
        });

        Ok(Self {
            port,
            access_code,
            shutdown,
            handle: Some(handle),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn jvm_args(&self) -> Vec<String> {
        vec![
            format!("-D{HOST_PROPERTY}={BRIDGE_HOST}"),
            format!("-D{PORT_PROPERTY}={}", self.port),
            format!("-D{CODE_PROPERTY}={}", self.access_code),
            format!("-D{PATH_PROPERTY}={SESSION_TOKEN_PATH}"),
        ]
    }
}

impl Drop for SessionBridge {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect((BRIDGE_HOST, self.port));

        if let Some(handle) = self.handle.take() {
            if let Err(error) = handle.join() {
                warn!(?error, "SayKOCraft session bridge thread panicked");
            }
        }
    }
}

impl fmt::Debug for SessionBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionBridge")
            .field("port", &self.port)
            .field("access_code", &"<redacted>")
            .finish_non_exhaustive()
    }
}

fn run_bridge(
    listener: TcpListener,
    session_token: String,
    expected_access_code: String,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        match listener.accept() {
            Ok((stream, address)) => {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }

                if !address.ip().is_loopback() {
                    warn!(%address, "Rejected non-loopback SayKOCraft session bridge request");
                    continue;
                }

                match handle_bridge_request(stream, &expected_access_code, &session_token) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => {
                        warn!(%error, "Failed to handle SayKOCraft session bridge request");
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                warn!(%error, "SayKOCraft session bridge listener failed");
                break;
            }
        }
    }

    debug!("SayKOCraft session bridge stopped");
}

fn handle_bridge_request(
    mut stream: TcpStream,
    expected_access_code: &str,
    session_token: &str,
) -> io::Result<bool> {
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;

    let request = read_http_request(&mut stream)?;
    let Some((method, path, access_code)) = parse_bridge_request(&request) else {
        write_response(&mut stream, 400, "Bad Request", "text/plain", "bad request")?;
        return Ok(false);
    };

    if method != "GET" {
        write_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "text/plain",
            "method not allowed",
        )?;
        return Ok(false);
    }

    if path != SESSION_TOKEN_PATH {
        write_response(&mut stream, 404, "Not Found", "text/plain", "not found")?;
        return Ok(false);
    }

    if access_code.as_deref() != Some(expected_access_code) {
        write_response(
            &mut stream,
            401,
            "Unauthorized",
            "text/plain",
            "unauthorized",
        )?;
        return Ok(false);
    }

    let body = json!({ "sessionToken": session_token }).to_string();
    write_response(&mut stream, 200, "OK", "application/json", &body)?;
    Ok(true)
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<String> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];

    loop {
        let bytes_read = stream.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..bytes_read]);
        if request.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session bridge request is too large",
            ));
        }

        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    String::from_utf8(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

fn parse_bridge_request(request: &str) -> Option<(&str, &str, Option<&str>)> {
    let mut lines = request.lines();
    let request_line = lines.next()?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next()?;
    let path = request_parts.next()?.split('?').next()?;
    let access_code = header_value(lines, ACCESS_CODE_HEADER);

    Some((method, path, access_code))
}

fn header_value<'a>(
    lines: impl Iterator<Item = &'a str>,
    expected_header_name: &str,
) -> Option<&'a str> {
    for line in lines {
        if line.trim().is_empty() {
            break;
        }

        let Some((name, value)) = line.split_once(':') else {
            continue;
        };

        if name.trim().eq_ignore_ascii_case(expected_header_name) {
            return Some(value.trim());
        }
    }

    None
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Cache-Control: no-store\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )?;
    stream.flush()
}

fn generate_access_code() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to generate session bridge access code: {error}"),
        )
    })?;

    Ok(hex::encode(bytes))
}

//! ureq's API over the browser's synchronous XMLHttpRequest.

use std::fmt;
use std::io::{Cursor, Read};
use std::time::Duration;

/// `ureq::Error`: a status of 400 or more with its response, or no response at all.
#[derive(Debug)]
pub enum Error {
    Status(u16, Response),
    Transport(Transport),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Status(code, response) => write!(f, "{}: status code {code}", response.url),
            Error::Transport(t) => t.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// Why no response came (a network error, a CORS refusal, a timeout).
#[derive(Debug)]
pub struct Transport {
    message: String,
}

impl Transport {
    /// What kind of failure: in the browser it can't tell a refused connection from a missing
    /// CORS header or an unknown host.
    pub fn kind(&self) -> ErrorKind {
        ErrorKind::Io
    }
}

/// `ureq::ErrorKind`, as far as the browser can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Io,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Network Error: the request didn't reach the server or wasn't allowed (CORS)")
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn transport(message: impl Into<String>) -> Error {
    Error::Transport(Transport {
        message: message.into(),
    })
}

#[derive(Clone, Debug, Default)]
pub struct AgentBuilder {
    timeout: Option<Duration>,
}

impl AgentBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    pub fn timeout_connect(self, _timeout: Duration) -> Self {
        self
    }
    pub fn timeout_read(mut self, timeout: Duration) -> Self {
        self.timeout = Some(self.timeout.map_or(timeout, |t| t.max(timeout)));
        self
    }
    /// Browsers don't let a page set `User-Agent`; kept for the API.
    pub fn user_agent(self, _agent: &str) -> Self {
        self
    }
    /// The browser follows redirects itself.
    pub fn redirects(self, _count: u32) -> Self {
        self
    }
    pub fn build(self) -> Agent {
        Agent {
            timeout: self.timeout,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Agent {
    timeout: Option<Duration>,
}

impl Agent {
    pub fn request(&self, method: &str, url: &str) -> Request {
        Request {
            method: method.to_owned(),
            url: url.to_owned(),
            headers: Vec::new(),
            timeout: self.timeout,
        }
    }
    pub fn get(&self, url: &str) -> Request {
        self.request("GET", url)
    }
    pub fn post(&self, url: &str) -> Request {
        self.request("POST", url)
    }
    pub fn put(&self, url: &str) -> Request {
        self.request("PUT", url)
    }
    pub fn delete(&self, url: &str) -> Request {
        self.request("DELETE", url)
    }
    pub fn head(&self, url: &str) -> Request {
        self.request("HEAD", url)
    }
}

pub fn get(url: &str) -> Request {
    Agent::default().get(url)
}
pub fn post(url: &str) -> Request {
    Agent::default().post(url)
}
pub fn request(method: &str, url: &str) -> Request {
    Agent::default().request(method, url)
}

#[derive(Clone, Debug)]
pub struct Request {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
}

impl Request {
    pub fn set(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    pub fn call(self) -> Result<Response, Error> {
        self.perform(Body::None)
    }
    pub fn send_string(self, body: &str) -> Result<Response, Error> {
        self.perform(Body::Text(body))
    }
    pub fn send_bytes(self, body: &[u8]) -> Result<Response, Error> {
        self.perform(Body::Bytes(body))
    }
    /// Sends what `reader` gives, read whole first: the browser takes a request body at once.
    pub fn send(self, mut reader: impl Read) -> Result<Response, Error> {
        let mut body = Vec::new();
        reader
            .read_to_end(&mut body)
            .map_err(|e| transport(format!("reading the body for {}: {e}", self.url)))?;
        self.perform(Body::Bytes(&body))
    }

    fn perform(self, body: Body<'_>) -> Result<Response, Error> {
        use web_sys::XmlHttpRequest;
        let failed =
            |what: &str, e: wasm_bindgen::JsValue| transport(format!("{what} {}: {e:?}", self.url));
        let xhr = XmlHttpRequest::new().map_err(|e| failed("XMLHttpRequest", e))?;
        xhr.open_with_async(&self.method, &self.url, false)
            .map_err(|e| failed("open", e))?;
        // Bytes as they came: each char of the text is one byte (0xF700 + b in some engines).
        xhr.override_mime_type("text/plain; charset=x-user-defined")
            .map_err(|e| failed("overrideMimeType", e))?;
        if let Some(timeout) = self.timeout {
            // Only workers allow a timeout on a synchronous request; the window throws, and waits.
            xhr.set_timeout(timeout.as_millis().min(u32::MAX as u128) as u32);
        }
        for (name, value) in &self.headers {
            if !name.eq_ignore_ascii_case("user-agent") {
                let _ = xhr.set_request_header(name, value);
            }
        }
        match body {
            Body::None => xhr.send(),
            Body::Text(text) => xhr.send_with_opt_str(Some(text)),
            // Copied out of the shared memory: the browser takes no view of it.
            Body::Bytes(bytes) => {
                xhr.send_with_opt_buffer_source(Some(&js_sys::Uint8Array::from(bytes)))
            }
        }
        .map_err(|e| failed("send", e))?;
        let status = xhr.status().map_err(|e| failed("status", e))?;
        if status == 0 {
            return Err(transport(format!(
                "{} {}: no response (network or CORS)",
                self.method, self.url
            )));
        }
        let text = xhr.response_text().ok().flatten().unwrap_or_default();
        let body: Vec<u8> = text.chars().map(|c| (c as u32 & 0xff) as u8).collect();
        let headers = xhr
            .get_all_response_headers()
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();
        let response = Response {
            url: self.url,
            status,
            headers,
            body,
        };
        if status >= 400 {
            return Err(Error::Status(status, response));
        }
        Ok(response)
    }
}

enum Body<'a> {
    None,
    Text(&'a str),
    Bytes(&'a [u8]),
}

#[derive(Debug)]
pub struct Response {
    url: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn get_url(&self) -> &str {
        &self.url
    }
    /// The media type without its parameters, `text/plain` when the response has none.
    pub fn content_type(&self) -> &str {
        self.header("Content-Type")
            .and_then(|t| t.split(';').next())
            .map(str::trim)
            .unwrap_or("text/plain")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn into_string(self) -> std::io::Result<String> {
        String::from_utf8(self.body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    pub fn into_reader(self) -> Box<dyn Read + Send + Sync + 'static> {
        Box::new(Cursor::new(self.body))
    }
}

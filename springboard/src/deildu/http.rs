//! HTTP/1.1 over rustls: keep-alive connections, redirects, a cookie jar
//! and byte ranges. The transport is a trait so tests run on fake sockets.

use super::dns::Resolver;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use rustls::client::danger::ServerCertVerifier as _;
use url::Url;

const USER_AGENT: &str = "Mozilla/5.0 (bifrost; CPU OS 15_7 like Golden Gate) WebKit/605.1.15 (KHTML, like Gecko) Version/15.6 Mobile/15E148 Browser/604.1";
const MAX_REDIRECTS: usize = 5;
const IDLE_PER_HOST: usize = 4;
const HEADER_LIMIT: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const READ_TIMEOUT: Duration = Duration::from_secs(25);

pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// Opens connections; the real one does DNS + TCP + TLS.
pub trait Transport: Send + Sync {
    fn connect(&self, host: &str, port: u16, tls: bool) -> io::Result<Box<dyn Stream>>;
    /// Wraps an open plain connection in TLS for `host` (STARTTLS). Transports without TLS refuse.
    fn upgrade(&self, host: &str, stream: Box<dyn Stream>) -> io::Result<Box<dyn Stream>> {
        let _ = (host, stream);
        Err(io::Error::other("this transport cannot upgrade to TLS"))
    }
}

/// A server certificate chain that the TLS stack refused: the host asked for, the DER of each certificate (the
/// server's own first) and a sentence for the person that says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsRejection {
    pub host: String,
    pub chain: Vec<Vec<u8>>,
    pub reason: String,
}

/// The most refused chains kept at once; the oldest goes first.
const MAX_REJECTIONS: usize = 8;

static REJECTIONS: Mutex<Vec<TlsRejection>> = Mutex::new(Vec::new());

/// Remembers a refused chain for `r.host`, replacing an earlier one for the same host.
pub fn record_rejection(r: TlsRejection) {
    insert_rejection(&mut REJECTIONS.lock().unwrap_or_else(|e| e.into_inner()), r);
}

fn insert_rejection(all: &mut Vec<TlsRejection>, r: TlsRejection) {
    all.retain(|x| !x.host.eq_ignore_ascii_case(&r.host));
    all.push(r);
    if all.len() > MAX_REJECTIONS {
        all.remove(0);
    }
}

/// Takes the refused chain remembered for `host`, if the TLS stack refused one since the last call.
pub fn take_rejection(host: &str) -> Option<TlsRejection> {
    let mut all = REJECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let i = all.iter().position(|x| x.host.eq_ignore_ascii_case(host))?;
    Some(all.remove(i))
}

/// The sentence for why a chain was refused.
fn rejection_reason(e: &rustls::Error) -> &'static str {
    use rustls::CertificateError as C;
    match e {
        rustls::Error::InvalidCertificate(c) => match c {
            C::Expired | C::ExpiredContext { .. } => "The certificate has expired.",
            C::NotValidYet | C::NotValidYetContext { .. } => "The certificate is not valid yet.",
            C::UnknownIssuer => "The certificate was issued by an authority this system does not trust.",
            C::NotValidForName | C::NotValidForNameContext { .. } => "The certificate was not issued for this site.",
            C::Revoked => "The certificate was revoked by its issuer.",
            C::BadSignature => "The signature on the certificate is not valid.",
            _ => "The certificate could not be verified.",
        },
        _ => "The secure connection could not be set up.",
    }
}

/// The webpki verifier, plus a record of every certificate chain it refuses (see [`take_rejection`]).
#[derive(Debug)]
struct RecordingVerifier {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
}

impl rustls::client::danger::ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let verdict = self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now);
        if let Err(e @ rustls::Error::InvalidCertificate(_)) = &verdict {
            let chain = std::iter::once(end_entity).chain(intermediates).map(|c| c.as_ref().to_vec()).collect();
            record_rejection(TlsRejection { host: server_name.to_str().into_owned(), chain, reason: rejection_reason(e).to_string() });
        }
        verdict
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// TCP with rustls (graviola provider, webpki roots) through the UDP resolver.
pub struct Tls {
    config: Arc<rustls::ClientConfig>,
    resolver: Resolver,
}

impl Tls {
    pub fn new() -> Self {
        let roots = Arc::new(rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() });
        let provider = Arc::new(rustls_graviola::default_provider());
        let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
            .build()
            .expect("the webpki roots are a valid trust store");
        // The same verification as `with_root_certificates`; the wrapper only remembers a chain it refused, so the
        // shell can show it. It never turns a refusal into an acceptance.
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("graviola supports TLS 1.2 and 1.3")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(RecordingVerifier { inner }))
            .with_no_client_auth();
        Tls { config: Arc::new(config), resolver: Resolver::default() }
    }
}

impl Default for Tls {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for Tls {
    fn connect(&self, host: &str, port: u16, tls: bool) -> io::Result<Box<dyn Stream>> {
        let mut err = io::Error::other("no address");
        for ip in self.resolver.resolve(host)? {
            match TcpStream::connect_timeout(&SocketAddr::from((ip, port)), CONNECT_TIMEOUT) {
                Ok(sock) => {
                    sock.set_read_timeout(Some(READ_TIMEOUT))?;
                    sock.set_write_timeout(Some(READ_TIMEOUT))?;
                    sock.set_nodelay(true)?;
                    if !tls {
                        return Ok(Box::new(sock));
                    }
                    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| io::Error::other("bad host name"))?;
                    let conn = rustls::ClientConnection::new(self.config.clone(), name).map_err(io::Error::other)?;
                    return Ok(Box::new(rustls::StreamOwned::new(conn, sock)));
                }
                Err(e) => err = e,
            }
        }
        Err(err)
    }

    fn upgrade(&self, host: &str, stream: Box<dyn Stream>) -> io::Result<Box<dyn Stream>> {
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| io::Error::other("bad host name"))?;
        let conn = rustls::ClientConnection::new(self.config.clone(), name).map_err(io::Error::other)?;
        Ok(Box::new(rustls::StreamOwned::new(conn, stream)))
    }
}

/// One cookie as the site set it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// without a leading dot
    pub domain: String,
    pub secure: bool,
}

/// In-memory cookie jar with a plain-text save format (one cookie per line).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Jar {
    cookies: Vec<Cookie>,
}

impl Jar {
    /// Applies a Set-Cookie header received from `host`.
    pub fn set(&mut self, host: &str, header: &str) {
        let mut parts = header.split(';').map(str::trim);
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else { return };
        let (name, value) = (name.trim().to_string(), value.trim().to_string());
        if name.is_empty() {
            return;
        }
        let (mut domain, mut secure, mut delete) = (host.to_string(), false, false);
        for attr in parts {
            let (k, v) = attr.split_once('=').map_or((attr, ""), |(k, v)| (k.trim(), v.trim()));
            match k.to_ascii_lowercase().as_str() {
                "domain" if !v.is_empty() => domain = v.trim_start_matches('.').to_ascii_lowercase(),
                "secure" => secure = true,
                "max-age" => delete = v.parse::<i64>().is_ok_and(|n| n <= 0),
                _ => {}
            }
        }
        if !matches_domain(host, &domain) {
            return;
        }
        self.cookies.retain(|c| !(c.name == name && c.domain == domain));
        if !delete {
            self.cookies.push(Cookie { name, value, domain, secure });
        }
    }

    /// The Cookie header value for a request, if any cookie applies.
    pub fn header(&self, host: &str, secure: bool) -> Option<String> {
        let v: Vec<String> = self.cookies.iter().filter(|c| matches_domain(host, &c.domain) && (secure || !c.secure)).map(|c| format!("{}={}", c.name, c.value)).collect();
        (!v.is_empty()).then(|| v.join("; "))
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    pub fn save(&self) -> String {
        self.cookies.iter().map(|c| format!("{}\t{}\t{}\t{}\n", c.domain, c.secure as u8, c.name, c.value)).collect()
    }

    pub fn load(text: &str) -> Jar {
        let cookies = text
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.splitn(4, '\t').collect();
                (f.len() == 4).then(|| Cookie { domain: f[0].into(), secure: f[1] == "1", name: f[2].into(), value: f[3].into() })
            })
            .collect();
        Jar { cookies }
    }
}

fn matches_domain(host: &str, domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}

pub type Headers = Vec<(String, String)>;
/// status, headers, body
type Reply = (u16, Headers, Vec<u8>);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// where the body came from, after redirects
    pub url: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Body as text; Latin-1 when it is not UTF-8 (the site is old).
    pub fn text(&self) -> String {
        deildu_rust::listings::latin1(&self.body)
    }
}

/// Parses "HTTP/1.1 200 OK\r\nHeader: v\r\n\r\n" (without the body).
pub fn parse_head(head: &str) -> Option<(u16, Headers)> {
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    Some((status, headers))
}

/// How far a chunked body has arrived.
enum Chunks {
    Done(Vec<u8>),
    Incomplete,
    Bad,
}

/// An incremental chunked-body parser: `advance` is given the whole buffer
/// again after each read but resumes where it stopped, so a long body is
/// parsed once, not once per read. It never indexes out of range.
#[derive(Default)]
struct Chunked {
    out: Vec<u8>,
    /// start of the first item (chunk or trailer line) not yet consumed
    pos: usize,
    trailers: bool,
}

impl Chunked {
    fn advance(&mut self, data: &[u8]) -> Chunks {
        let crlf = |from: usize| -> Option<usize> { data.get(from..)?.windows(2).position(|w| w == b"\r\n").map(|p| p + from) };
        loop {
            let Some(line_end) = crlf(self.pos) else { return Chunks::Incomplete };
            if self.trailers {
                // trailers: header lines until an empty line
                let empty = line_end == self.pos;
                self.pos = line_end + 2;
                if empty {
                    return Chunks::Done(std::mem::take(&mut self.out));
                }
                continue;
            }
            let Ok(size_str) = std::str::from_utf8(&data[self.pos..line_end]) else { return Chunks::Bad };
            let Some(size) = size_str.split(';').next().and_then(|t| usize::from_str_radix(t.trim(), 16).ok()) else { return Chunks::Bad };
            let start = line_end + 2;
            if size == 0 {
                self.pos = start;
                self.trailers = true;
                continue;
            }
            let Some(end) = start.checked_add(size) else { return Chunks::Bad };
            if end.checked_add(2).map_or(true, |x| x > data.len()) {
                return Chunks::Incomplete;
            }
            if &data[end..end + 2] != b"\r\n" {
                return Chunks::Bad;
            }
            self.out.extend_from_slice(&data[start..end]);
            self.pos = end + 2;
        }
    }
}

/// Decodes a chunked body; None when malformed or truncated.
pub fn dechunk(data: &[u8]) -> Option<Vec<u8>> {
    match Chunked::default().advance(data) {
        Chunks::Done(v) => Some(v),
        _ => None,
    }
}

fn stream_line(r:&mut impl std::io::BufRead) -> io::Result<Vec<u8>> {
    let mut line=Vec::new();
    let mut limited=Read::by_ref(r).take((HEADER_LIMIT+1) as u64);
    std::io::BufRead::read_until(&mut limited,b'\n',&mut line)?;
    if line.len()>HEADER_LIMIT || !line.ends_with(b"\r\n") {return Err(io::Error::new(io::ErrorKind::UnexpectedEof,"invalid HTTP line"));}
    Ok(line)
}
fn stream_body(r:&mut impl std::io::BufRead,framing:Framing,limit:u64,sink:&mut impl FnMut(&[u8])->io::Result<()>) -> io::Result<()> {
    let mut buffer=[0u8;32*1024];let mut total=0u64;
    let mut copy=|r:&mut dyn Read,mut left:u64|->io::Result<()> {
        if total.checked_add(left).is_none_or(|n|n>limit) {return Err(io::Error::other("response body too large"));}
        while left>0 {let wanted=left.min(buffer.len() as u64) as usize;let n=r.read(&mut buffer[..wanted])?;
            if n==0 {return Err(io::Error::new(io::ErrorKind::UnexpectedEof,"truncated HTTP body"));}
            sink(&buffer[..n])?;left-=n as u64;total+=n as u64;
        }Ok(())
    };
    match framing {
        Framing::Length(n)=>copy(r,n as u64),
        Framing::Chunked=>{loop {
            let line=stream_line(r)?;
            let size=std::str::from_utf8(&line).ok().and_then(|s|u64::from_str_radix(s.trim().split(';').next().unwrap_or(""),16).ok()).ok_or_else(||io::Error::other("invalid chunk size"))?;
            if size==0 {let mut trailers=0;while stream_line(r)?!=b"\r\n" {trailers+=1;if trailers>128{return Err(io::Error::other("too many trailers"));}}return Ok(());}
            copy(r,size)?;let mut end=[0;2];r.read_exact(&mut end)?;if end!=*b"\r\n" {return Err(io::Error::other("invalid chunk terminator"));}
        }},
        Framing::Close=>{drop(copy);loop {let n=r.read(&mut buffer)?;if n==0 {return Ok(());}
            total+=n as u64;if total>limit {return Err(io::Error::other("response body too large"));}sink(&buffer[..n])?;}},
    }
}

/// Where a body ends, from the response headers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Framing {
    Length(usize),
    Chunked,
    Close,
}

fn framing(status: u16, headers: &[(String, String)], head_only: bool) -> Framing {
    if head_only || status / 100 == 1 || status == 204 || status == 304 {
        return Framing::Length(0);
    }
    let find = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str());
    if find("Transfer-Encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        Framing::Chunked
    } else if let Some(n) = find("Content-Length").and_then(|v| v.trim().parse().ok()) {
        Framing::Length(n)
    } else {
        Framing::Close
    }
}

/// Reads one response; `keep` reports whether the connection may be reused.
fn read_response(s: &mut dyn Stream, limit: usize, head_only: bool) -> io::Result<(Reply, bool)> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let mut scanned = 0;
    let head_len = loop {
        // only the new tail (plus the 3 bytes a terminator may straddle) needs a look
        if let Some(p) = buf[scanned..].windows(4).position(|w| w == b"\r\n\r\n") {
            break scanned + p + 4;
        }
        scanned = buf.len().saturating_sub(3);
        if buf.len() > HEADER_LIMIT {
            return Err(io::Error::other("response headers too large"));
        }
        let n = s.read(&mut chunk)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_len]).into_owned();
    let (status, headers) = parse_head(&head).ok_or_else(|| io::Error::other("bad status line"))?;
    let mut body = buf.split_off(head_len);
    let framing = framing(status, &headers, head_only);
    let close = headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("Connection") && v.eq_ignore_ascii_case("close")) || head.starts_with("HTTP/1.0");
    let mut read_until = |done: &mut dyn FnMut(&[u8]) -> bool, body: &mut Vec<u8>| -> io::Result<()> {
        while !done(body) {
            if body.len() > limit {
                return Err(io::Error::other("response body too large"));
            }
            let n = s.read(&mut chunk)?;
            if n == 0 {
                return Ok(());
            }
            body.extend_from_slice(&chunk[..n]);
        }
        Ok(())
    };
    let body = match framing {
        Framing::Length(n) => {
            if n > limit {
                return Err(io::Error::other("response body too large"));
            }
            read_until(&mut |b| b.len() >= n, &mut body)?;
            if body.len() < n {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short body"));
            }
            body.truncate(n);
            body
        }
        Framing::Chunked => {
            let mut parser = Chunked::default();
            let mut decoded = None;
            read_until(
                &mut |b| match parser.advance(b) {
                    Chunks::Incomplete => false,
                    Chunks::Done(v) => {
                        decoded = Some(v);
                        true
                    }
                    Chunks::Bad => true,
                },
                &mut body,
            )?;
            decoded.ok_or_else(|| io::Error::other("bad chunked body"))?
        }
        Framing::Close => {
            read_until(&mut |_| false, &mut body)?;
            body
        }
    };
    Ok(((status, headers, body), !close && framing != Framing::Close))
}

type Idle = HashMap<(String, u16), Vec<Box<dyn Stream>>>;

/// A browsing session: cookies, connection reuse and redirects.
pub struct Client {
    transport: Arc<dyn Transport>,
    jar: Mutex<Jar>,
    idle: Mutex<Idle>,
    https_only: bool,
}

impl Client {
    pub fn new(transport: Arc<dyn Transport>, jar: Jar) -> Self {
        Client { transport, jar: Mutex::new(jar), idle: Mutex::new(HashMap::new()), https_only: false }
    }

    /// Require TLS for the original request and every redirect.
    pub fn require_https(mut self) -> Self {
        self.https_only = true;
        self
    }

    /// Stream a package response through a bounded buffer; resume only a validated range.
    pub fn get_streamed(&self,url:&str,start:u64,limit:u64,mut sink:impl FnMut(&[u8])->io::Result<()>) -> io::Result<()> {
        let mut url=url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            let u=Url::parse(&url).map_err(|_|io::Error::other("bad url"))?;
            if self.https_only && u.scheme() != "https" { return Err(io::Error::other("HTTPS required, including redirects")); }
            let tls=u.scheme()=="https";
            if !tls && u.scheme()!="http" {return Err(io::Error::other("unsupported scheme"));}
            let host=u.host_str().ok_or_else(||io::Error::other("missing host"))?;
            let mut stream=self.transport.connect(host,u.port_or_known_default().unwrap_or(443),tls)?;
            let extra=if start>0 {vec![("Range".into(),format!("bytes={start}-"))]} else {vec![]};
            stream.write_all(&request_bytes("GET",&u,host,&extra,None,None))?;stream.flush()?;
            let mut reader=std::io::BufReader::new(stream);
            let mut head=Vec::new();
            loop {
                let line=stream_line(&mut reader)?;let empty=line==b"\r\n";head.extend_from_slice(&line);
                if head.len()>HEADER_LIMIT {return Err(io::Error::other("response headers too large"));}
                if empty {break;}
            }
            let (status,headers)=parse_head(&String::from_utf8_lossy(&head)).ok_or_else(||io::Error::other("bad status line"))?;
            if matches!(status,301|302|303|307|308) {
                let location=headers.iter().find(|(k,_)|k.eq_ignore_ascii_case("Location")).ok_or_else(||io::Error::other("missing redirect"))?;
                url=u.join(&location.1).map_err(io::Error::other)?.to_string();continue;
            }
            if start>0 && status==200 {return Err(io::Error::new(io::ErrorKind::Unsupported,"server does not support resume"));}
            if status!=200 && status!=206 {return Err(io::Error::other(format!("server answered {status}")));}
            if status==206 {
                let range=headers.iter().find(|(k,_)|k.eq_ignore_ascii_case("Content-Range")).and_then(|(_,v)|v.strip_prefix("bytes ")).and_then(|v|v.split_once('-')).and_then(|(v,_)|v.parse::<u64>().ok());
                if range!=Some(start) {return Err(io::Error::other("wrong response range"));}
            }
            return stream_body(&mut reader,framing(status,&headers,false),limit,&mut sink);
        }
        Err(io::Error::other("too many redirects"))
    }

    pub fn jar(&self) -> Jar {
        self.jar.lock().unwrap().clone()
    }

    pub fn set_jar(&self, jar: Jar) {
        *self.jar.lock().unwrap() = jar;
    }

    /// GET with an optional inclusive byte range and a body size limit.
    pub fn get(&self, url: &str, range: Option<(u64, u64)>, limit: usize) -> io::Result<Response> {
        let extra = range.map(|(a, b)| vec![("Range".to_string(), format!("bytes={a}-{b}"))]).unwrap_or_default();
        self.send("GET", url, extra, None, limit)
    }

    /// GET with caller-supplied headers (name, value) and a body size limit. A `User-Agent` header replaces the default one.
    pub fn get_with_headers(&self, url: &str, headers: &[(&str, &str)], limit: usize) -> io::Result<Response> {
        let extra = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.send("GET", url, extra, None, limit)
    }

    /// POST application/x-www-form-urlencoded.
    pub fn post_form(&self, url: &str, fields: &[(&str, &str)], limit: usize) -> io::Result<Response> {
        let body = form_encode(fields);
        let extra = vec![("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string())];
        self.send("POST", url, extra, Some(body.into_bytes()), limit)
    }

    /// POST `body` with caller-supplied headers (name, value). Used by the DAW's AI client (JSON + API key).
    pub fn post_with_headers(&self, url: &str, headers: &[(&str, &str)], body: &[u8], limit: usize) -> io::Result<Response> {
        let extra = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.send("POST", url, extra, Some(body.to_vec()), limit)
    }

    /// POST a text/plain body (diagnostics upload).
    pub fn post_text(&self, url: &str, body: &[u8], limit: usize) -> io::Result<Response> {
        let extra = vec![("Content-Type".to_string(), "text/plain".to_string())];
        self.send("POST", url, extra, Some(body.to_vec()), limit)
    }

    /// Any method with caller-supplied headers (name, value) and an optional body, for APIs that send a bearer token
    /// (Matrix uses GET, PUT and POST).
    pub fn request_with_headers(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, limit: usize) -> io::Result<Response> {
        let extra = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.send(method, url, extra, body.map(<[u8]>::to_vec), limit)
    }

    fn send(&self, method: &str, url: &str, extra: Vec<(String, String)>, body: Option<Vec<u8>>, limit: usize) -> io::Result<Response> {
        let (mut method, mut url, mut body) = (method.to_string(), url.to_string(), body);
        for _ in 0..=MAX_REDIRECTS {
            let u = Url::parse(&url).map_err(|_| io::Error::other("bad url"))?;
            let (status, headers, data) = self.once(&method, &u, &extra, body.as_deref(), limit)?;
            let location = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("Location")).map(|(_, v)| v.clone());
            match (status, location) {
                (301 | 302 | 303 | 307 | 308, Some(loc)) => {
                    url = u.join(&loc).map_err(|_| io::Error::other("bad redirect"))?.to_string();
                    if status == 303 || (status != 307 && status != 308 && method == "POST") {
                        method = "GET".into();
                        body = None;
                    }
                }
                _ => return Ok(Response { status, headers, body: data, url }),
            }
        }
        Err(io::Error::other("too many redirects"))
    }

    fn take_idle(&self, key: &(String, u16)) -> Option<Box<dyn Stream>> {
        self.idle.lock().unwrap().get_mut(key).and_then(Vec::pop)
    }

    fn park(&self, key: (String, u16), s: Box<dyn Stream>) {
        let mut idle = self.idle.lock().unwrap();
        let v = idle.entry(key).or_default();
        if v.len() < IDLE_PER_HOST {
            v.push(s);
        }
    }

    /// One exchange, retrying once on a fresh connection when a reused one died.
    fn once(&self, method: &str, u: &Url, extra: &[(String, String)], body: Option<&[u8]>, limit: usize) -> io::Result<Reply> {
        if self.https_only && u.scheme() != "https" { return Err(io::Error::other("HTTPS required, including redirects")); }
        let tls = u.scheme() == "https";
        let host = u.host_str().ok_or_else(|| io::Error::other("no host"))?.to_string();
        let port = u.port_or_known_default().unwrap_or(if tls { 443 } else { 80 });
        if !tls && u.scheme() != "http" {
            return Err(io::Error::other("unsupported scheme"));
        }
        let key = (host.clone(), port);
        let cookie = self.jar.lock().unwrap().header(&host, tls);
        let req = request_bytes(method, u, &host, extra, cookie.as_deref(), body);
        let mut reused = true;
        let mut attempt = 0;
        loop {
            let mut s = match self.take_idle(&key) {
                Some(s) => s,
                None => {
                    reused = false;
                    self.transport.connect(&host, port, tls)?
                }
            };
            let result = s.write_all(&req).and_then(|_| s.flush()).and_then(|_| read_response(&mut *s, limit, method == "HEAD"));
            match result {
                Ok(((status, headers, data), keep)) => {
                    let mut jar = self.jar.lock().unwrap();
                    for (k, v) in &headers {
                        if k.eq_ignore_ascii_case("Set-Cookie") {
                            jar.set(&host, v);
                        }
                    }
                    drop(jar);
                    if keep {
                        self.park(key, s);
                    }
                    return Ok((status, headers, data));
                }
                Err(e) if reused && attempt == 0 => {
                    attempt += 1;
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

fn request_bytes(method: &str, u: &Url, host: &str, extra: &[(String, String)], cookie: Option<&str>, body: Option<&[u8]>) -> Vec<u8> {
    let mut target = u.path().to_string();
    if let Some(q) = u.query() {
        target.push('?');
        target.push_str(q);
    }
    // a caller that names its own User-Agent (a tile or geocoder service that asks clients to identify themselves) replaces the default
    let own_agent = extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("User-Agent"));
    let agent = if own_agent { String::new() } else { format!("User-Agent: {USER_AGENT}\r\n") };
    let mut req = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n{agent}Accept: */*\r\nConnection: keep-alive\r\n");
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(c) = cookie {
        req.push_str(&format!("Cookie: {c}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    let mut out = req.into_bytes();
    if let Some(b) = body {
        out.extend_from_slice(b);
    }
    out
}

/// application/x-www-form-urlencoded.
pub fn form_encode(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect::<Vec<_>>().join("&")
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl deildu_rust::stream::Http for Client {
    fn get(&self, url: &str, range: Option<(u64, u64)>, limit: usize) -> Result<deildu_rust::stream::Response, String> {
        let r = Client::get(self, url, range, limit).map_err(|e| e.to_string())?;
        let content_range = r.header("Content-Range").unwrap_or("").to_string();
        Ok(deildu_rust::stream::Response { status: r.status as usize, body: r.body, final_url: r.url, content_range })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A canned-reply transport. Requests whose "METHOD path" starts with a
    /// route prefix get that route's reply (reusable, on any connection);
    /// the rest consume `replies` in order on the connection that took them.
    pub struct Fake {
        pub replies: Mutex<VecDeque<Vec<u8>>>,
        pub requests: Arc<Mutex<Vec<String>>>,
        pub connects: Mutex<usize>,
        routes: Routes,
    }

    type Routes = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    struct FakeStream {
        requests: Arc<Mutex<Vec<String>>>,
        replies: Arc<Mutex<VecDeque<Vec<u8>>>>,
        routes: Routes,
        out: VecDeque<u8>,
    }

    impl Read for FakeStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(self.out.len());
            for b in buf.iter_mut().take(n) {
                *b = self.out.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    impl Write for FakeStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let req = String::from_utf8_lossy(buf).into_owned();
            let line = req.lines().next().unwrap_or("").to_string();
            self.requests.lock().unwrap().push(req);
            let routed = self.routes.lock().unwrap().iter().find(|(p, _)| line.starts_with(p.as_str())).map(|(_, r)| r.clone());
            match routed.or_else(|| self.replies.lock().unwrap().pop_front()) {
                Some(r) => self.out.extend(r),
                None => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "server gone")),
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Fake {
        pub fn new(replies: Vec<&str>) -> Arc<Self> {
            Arc::new(Fake { replies: Mutex::new(replies.into_iter().map(|r| r.as_bytes().to_vec()).collect()), requests: Default::default(), connects: Mutex::new(0), routes: Default::default() })
        }

        /// Serves `reply` for every request whose request line starts with `prefix` (e.g. "GET /a.jpg").
        pub fn route(&self, prefix: &str, reply: &str) {
            self.route_bytes(prefix, reply.as_bytes().to_vec());
        }

        pub fn route_bytes(&self, prefix: &str, reply: Vec<u8>) {
            self.routes.lock().unwrap().push((prefix.to_string(), reply));
        }

        /// A 200 reply with a binary body.
        pub fn ok_bytes(body: &[u8]) -> Vec<u8> {
            let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
            r.extend_from_slice(body);
            r
        }
    }

    impl Transport for Fake {
        fn connect(&self, _host: &str, _port: u16, _tls: bool) -> io::Result<Box<dyn Stream>> {
            *self.connects.lock().unwrap() += 1;
            let replies = Arc::new(Mutex::new(std::mem::take(&mut *self.replies.lock().unwrap())));
            Ok(Box::new(FakeStream { requests: self.requests.clone(), replies, routes: self.routes.clone(), out: VecDeque::new() }))
        }
    }

    pub fn ok(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    #[test]
    fn jar_matches_domains_and_deletes() {
        let mut j = Jar::default();
        j.set("www.example.net", "uid=1; Path=/; HttpOnly");
        j.set("www.example.net", "pass=abc; Domain=.example.net; Secure");
        j.set("www.example.net", "evil=1; Domain=other.net");
        assert_eq!(j.header("www.example.net", true).unwrap(), "uid=1; pass=abc");
        assert_eq!(j.header("www.example.net", false).unwrap(), "uid=1");
        assert_eq!(j.header("api.example.net", true).unwrap(), "pass=abc");
        assert_eq!(j.header("example.com", true), None);
        j.set("www.example.net", "uid=; Max-Age=0");
        assert_eq!(j.header("www.example.net", false), None);
        let copy = Jar::load(&j.save());
        assert_eq!(copy, j);
        j.clear();
        assert!(j.is_empty());
    }

    #[test]
    fn parses_heads_and_chunks() {
        let (status, h) = parse_head("HTTP/1.1 302 Found\r\nLocation: /x\r\nSet-Cookie: a=b\r\n").unwrap();
        assert_eq!(status, 302);
        assert_eq!(h[0], ("Location".to_string(), "/x".to_string()));
        assert_eq!(dechunk(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(), b"Wikipedia");
        assert!(dechunk(b"zz\r\n").is_none());
        assert_eq!(form_encode(&[("user", "a b&c"), ("pw", "é")]), "user=a+b%26c&pw=%C3%A9");
    }

    #[test]
    fn chunks_parse_incrementally_as_the_buffer_grows() {
        let wire = b"4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nTrailer: x\r\n\r\n";
        let mut parser = Chunked::default();
        for n in 0..wire.len() {
            assert!(matches!(parser.advance(&wire[..n]), Chunks::Incomplete), "complete too early at {n}");
        }
        assert!(matches!(parser.advance(wire), Chunks::Done(v) if v == b"Wikipedia"));
        assert!(matches!(Chunked::default().advance(b"zz\r\n"), Chunks::Bad));
    }

    #[test]
    fn secure_client_refuses_downgrade_before_connecting() {
        for mode in 0..3 {
            let fake = Fake::new(vec!["HTTP/1.1 307 Temporary Redirect\r\nLocation: http://insecure.example/data\r\nContent-Length: 0\r\n\r\n"]);
            let c = Client::new(fake.clone(), Jar::default()).require_https();
            let result = match mode {
                0 => c.get("https://secure.example/data", None, 1024).map(|_| ()),
                1 => c.post_text("https://secure.example/data", b"private", 1024).map(|_| ()),
                _ => c.get_streamed("https://secure.example/data", 0, 1024, |_| Ok(())),
            };
            assert!(result.is_err());
            assert_eq!(*fake.connects.lock().unwrap(), 1);
            assert!(c.get("http://insecure.example", None, 1024).is_err());
            assert_eq!(*fake.connects.lock().unwrap(), 1);
        }
    }

    #[test]
    fn get_follows_redirects_sends_cookies_and_reuses_the_connection() {
        let fake = Fake::new(vec!["HTTP/1.1 302 Found\r\nLocation: /home\r\nSet-Cookie: sid=42; Path=/\r\nContent-Length: 0\r\n\r\n", &ok("<html>home</html>")]);
        let c = Client::new(fake.clone(), Jar::default());
        let r = c.get("https://www.example.net/login?x=1", None, 1 << 20).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"<html>home</html>");
        assert_eq!(r.url, "https://www.example.net/home");
        let reqs = fake.requests.lock().unwrap().clone();
        assert!(reqs[0].starts_with("GET /login?x=1 HTTP/1.1\r\nHost: www.example.net\r\n"));
        assert!(reqs[1].contains("Cookie: sid=42\r\n"), "{}", reqs[1]);
        assert_eq!(*fake.connects.lock().unwrap(), 1, "keep-alive reused the socket");
        assert_eq!(c.jar().header("www.example.net", true).unwrap(), "sid=42");
    }

    #[test]
    fn a_named_user_agent_replaces_the_default_one() {
        let fake = Fake::new(vec![&ok("tile")]);
        let c = Client::new(fake.clone(), Jar::default());
        let r = c.get_with_headers("https://tile.example.net/1/2/3.png", &[("User-Agent", "Maps/1.0 (contact)")], 1024).unwrap();
        assert_eq!(r.body, b"tile");
        let reqs = fake.requests.lock().unwrap().clone();
        assert_eq!(reqs[0].matches("User-Agent:").count(), 1, "{}", reqs[0]);
        assert!(reqs[0].contains("User-Agent: Maps/1.0 (contact)\r\n"));
    }

    #[test]
    fn post_becomes_get_after_302_and_a_dead_idle_socket_is_replaced() {
        let fake = Fake::new(vec!["HTTP/1.1 302 Found\r\nLocation: /my\r\nContent-Length: 0\r\n\r\n", &ok("mine")]);
        let c = Client::new(fake.clone(), Jar::default());
        let r = c.post_form("https://h.net/takelogin", &[("u", "me"), ("p", "x y")], 1024).unwrap();
        assert_eq!(r.body, b"mine");
        let reqs = fake.requests.lock().unwrap().clone();
        assert!(reqs[0].starts_with("POST /takelogin HTTP/1.1"));
        assert!(reqs[0].ends_with("\r\n\r\nu=me&p=x+y"));
        assert!(reqs[0].contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(reqs[1].starts_with("GET /my HTTP/1.1"));
        // the parked socket has no replies left: the next request must reconnect
        fake.replies.lock().unwrap().push_back(ok("again").into_bytes());
        let r = c.get("https://h.net/again", None, 1024).unwrap();
        assert_eq!(r.body, b"again");
        assert_eq!(*fake.connects.lock().unwrap(), 2);
    }

    #[test]
    fn chunked_close_delimited_and_oversized_bodies() {
        let fake = Fake::new(vec![
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nrest",
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789",
        ]);
        let c = Client::new(fake.clone(), Jar::default());
        assert_eq!(c.get("http://h/a", None, 100).unwrap().body, b"abc");
        assert_eq!(c.get("http://h/b", None, 100).unwrap().body, b"rest");
        assert!(c.get("http://h/c", None, 5).is_err(), "limit enforced");
        assert_eq!(*fake.connects.lock().unwrap(), 2, "chunked keep-alive reused; close-delimited not");
    }

    #[test]
    fn range_requests_map_to_the_engine_trait() {
        use deildu_rust::stream::Http;
        let fake = Fake::new(vec!["HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-3/100\r\nContent-Length: 4\r\n\r\nabcd"]);
        let c = Client::new(fake.clone(), Jar::default());
        let r = Http::get(&c, "https://seed/x.mp4", Some((0, 3)), 4).unwrap();
        assert_eq!((r.status, r.content_range.as_str(), r.body.as_slice()), (206, "bytes 0-3/100", &b"abcd"[..]));
        assert!(fake.requests.lock().unwrap()[0].contains("Range: bytes=0-3\r\n"));
    }

    #[test]
    fn tls_transport_builds_and_rejects_unresolvable_hosts() {
        let t = Tls::new();
        assert!(t.connect("127.0.0.1", 9, true).is_err(), "nothing listens on the discard port");
    }

    #[test]
    fn a_chain_the_webpki_verifier_refuses_is_remembered_for_the_shell_and_still_refused() {
        use rustls::client::danger::ServerCertVerifier;
        let roots = Arc::new(rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() });
        let provider = Arc::new(rustls_graviola::default_provider());
        let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider).build().unwrap();
        let verifier = RecordingVerifier { inner };
        let der = crate::shell::trust_sheet::test_certificate("self-signed.test", "self-signed.test", 1_000_000_000, 2_000_000_000);
        let cert = rustls::pki_types::CertificateDer::from(der.clone());
        let name = rustls::pki_types::ServerName::try_from("self-signed.test").unwrap();
        let now = rustls::pki_types::UnixTime::since_unix_epoch(Duration::from_secs(1_500_000_000));
        let verdict = verifier.verify_server_cert(&cert, &[], &name, &[], now);
        assert!(verdict.is_err(), "a certificate no authority signed is refused, whatever is recorded");
        let refused = take_rejection("SELF-SIGNED.test").expect("the refusal is remembered, whatever the case of the host");
        assert_eq!((refused.host.as_str(), refused.chain), ("self-signed.test", vec![der]));
        assert!(!refused.reason.is_empty());
        assert!(take_rejection("self-signed.test").is_none(), "taken once");
    }

    #[test]
    fn only_the_newest_refusals_are_kept_and_a_host_has_one() {
        let mut all = Vec::new();
        let refusal = |host: String, reason: &str| TlsRejection { host, chain: vec![], reason: reason.into() };
        for i in 0..(MAX_REJECTIONS + 3) {
            insert_rejection(&mut all, refusal(format!("many-{i}.test"), "r"));
        }
        assert_eq!(all.len(), MAX_REJECTIONS);
        assert_eq!(all[0].host, "many-3.test", "the oldest went first");
        insert_rejection(&mut all, refusal("MANY-5.test".into(), "again"));
        assert_eq!(all.len(), MAX_REJECTIONS, "a host's later refusal replaces its earlier one");
        assert_eq!(all.last().unwrap().reason, "again");
    }

    #[test]
    fn the_reason_names_what_was_wrong_with_the_certificate() {
        use rustls::CertificateError as C;
        let reason = |c: C| rejection_reason(&rustls::Error::InvalidCertificate(c));
        assert!(reason(C::Expired).contains("expired"));
        assert!(reason(C::UnknownIssuer).contains("authority"));
        assert!(reason(C::NotValidForName).contains("not issued for this site"));
        assert!(reason(C::BadEncoding).contains("could not be verified"));
        assert!(rejection_reason(&rustls::Error::HandshakeNotComplete).contains("connection"));
    }
}

#[cfg(test)]
mod streaming_review_tests {
    use super::*;
    #[test]
    fn review_streaming_chunked_body_and_truncation() {
        let mut output=Vec::new();
        stream_body(&mut io::Cursor::new(b"3\r\nabc\r\n2\r\nde\r\n0\r\nX-Test: ok\r\n\r\n"),Framing::Chunked,5,&mut |b|{output.extend_from_slice(b);Ok(())}).unwrap();
        assert_eq!(output,b"abcde");
        let mut output=Vec::new();
        assert!(stream_body(&mut io::Cursor::new(b"abc"),Framing::Length(5),5,&mut |b|{output.extend_from_slice(b);Ok(())}).is_err());
        assert_eq!(output,b"abc");
        assert!(stream_body(&mut io::Cursor::new(b"abc"),Framing::Length(3),2,&mut |_|Ok(())).is_err());
    }
    #[test]
    fn review_streaming_large_body_has_bounded_sink_chunks() {
        let data=vec![9;2*1024*1024];let mut total=0;
        stream_body(&mut io::Cursor::new(&data),Framing::Length(data.len()),data.len() as u64,&mut |b| {assert!(b.len()<=32*1024);total+=b.len();Ok(())}).unwrap();
        assert_eq!(total,data.len());
    }
}

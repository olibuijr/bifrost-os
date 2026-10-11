//! Service request assembly (`SLRequest` parity, see `docs/parity/social.md`): an app builds the HTTP request for a web
//! service from a method, a URL, parameters and optional uploads, then sends it with `URLSession`.
//!
//! The rules follow the framework's request object:
//! - GET and DELETE send the parameters in the query string (joined with `?` or `&`).
//! - POST and PUT send the parameters as an `application/x-www-form-urlencoded` body. With uploads they become text
//!   fields of a `multipart/form-data` body (see [`crate::multipart`]), followed by the upload parts in the order added.
//! - Uploads need POST or PUT; any other method is rejected.
//!
//! Not mirrored: the service type (no named networks), the signed-in account (`account`) and the handler, which is the
//! `URLSession` completion. Requests are not signed, so a service that needs a token gets one from the app.

use crate::multipart::MultipartBody;
use crate::url::URLRequest;

/// `SLRequestMethod`: the HTTP methods the request object supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestMethod {
    Get,
    Post,
    Delete,
    Put,
}

impl RequestMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestMethod::Get => "GET",
            RequestMethod::Post => "POST",
            RequestMethod::Delete => "DELETE",
            RequestMethod::Put => "PUT",
        }
    }

    /// Whether the method sends its parameters in a body (POST and PUT) rather than in the query string.
    pub fn has_body(self) -> bool {
        matches!(self, RequestMethod::Post | RequestMethod::Put)
    }
}

/// An HTTP request for a web service, assembled before it is sent.
#[derive(Clone, Debug)]
pub struct ServiceRequest {
    pub method: RequestMethod,
    pub url: String,
    pub parameters: Vec<(String, String)>,
    uploads: MultipartBody,
}

impl ServiceRequest {
    /// `init(forServiceType:requestMethod:url:parameters:)` without the service type.
    pub fn new(method: RequestMethod, url: &str, parameters: &[(&str, &str)]) -> Self {
        ServiceRequest {
            method,
            url: url.to_string(),
            parameters: parameters.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            uploads: MultipartBody::new(),
        }
    }

    /// `addMultipartData(_:withName:type:filename:)`: adds an upload to the POST or PUT body.
    pub fn add_multipart_data(&mut self, data: &[u8], name: &str, content_type: &str, filename: Option<&str>) {
        self.uploads.add_data(data, name, content_type, filename);
    }

    /// `preparedURLRequest()`: the URL request to send with `URLSession`.
    pub fn prepared_url_request(&self) -> Result<URLRequest, &'static str> {
        let mut req = URLRequest::new(&self.url);
        req.method = self.method.as_str().to_string();
        if !self.method.has_body() {
            if !self.uploads.is_empty() {
                return Err("multipart data needs POST or PUT");
            }
            if !self.parameters.is_empty() {
                let sep = if self.url.contains('?') { '&' } else { '?' };
                req.url = format!("{}{sep}{}", self.url, encode_pairs(&self.parameters, false));
            }
            return Ok(req);
        }
        if self.uploads.is_empty() {
            if !self.parameters.is_empty() {
                req.body = Some(encode_pairs(&self.parameters, true).into_bytes());
                req.set_header("Content-Type", "application/x-www-form-urlencoded");
            }
            return Ok(req);
        }
        let mut body = MultipartBody::new();
        for (k, v) in &self.parameters {
            body.add_field(k, v);
        }
        for p in self.uploads.parts() {
            body.add_part(p.clone());
        }
        req.set_header("Content-Type", &body.content_type());
        req.body = Some(body.to_bytes());
        Ok(req)
    }
}

/// `k=v` pairs joined with `&`. Form bodies write a space as `+`; query strings write it as `%20`.
fn encode_pairs(pairs: &[(String, String)], form: bool) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k, form), encode(v, form))).collect::<Vec<_>>().join("&")
}

/// Percent-encodes everything except the RFC 3986 unreserved characters.
fn encode(s: &str, form: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' if form => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_puts_parameters_in_the_query_string() {
        let r = ServiceRequest::new(RequestMethod::Get, "https://example.test/feed", &[("q", "a b"), ("n", "5")]);
        let req = r.prepared_url_request().unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "https://example.test/feed?q=a%20b&n=5");
        assert!(req.body.is_none());
    }

    #[test]
    fn query_joins_with_ampersand_when_the_url_has_one() {
        let r = ServiceRequest::new(RequestMethod::Delete, "https://example.test/item?id=2", &[("force", "1")]);
        let req = r.prepared_url_request().unwrap();
        assert_eq!(req.method, "DELETE");
        assert_eq!(req.url, "https://example.test/item?id=2&force=1");
    }

    #[test]
    fn post_with_parameters_sends_a_form_body() {
        let r = ServiceRequest::new(RequestMethod::Post, "https://example.test/post", &[("text", "hi & bye+"), ("x", "é")]);
        let req = r.prepared_url_request().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://example.test/post");
        assert_eq!(req.body.as_deref(), Some(&b"text=hi+%26+bye%2B&x=%C3%A9"[..]));
        assert!(req.headers.contains(&("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string())));
    }

    #[test]
    fn post_with_an_upload_sends_fields_then_files_as_multipart() {
        let mut r = ServiceRequest::new(RequestMethod::Put, "https://example.test/media", &[("caption", "sun")]);
        r.add_multipart_data(b"\x89PNG", "image", "image/png", Some("sun.png"));
        let req = r.prepared_url_request().unwrap();
        assert_eq!(req.method, "PUT");
        let content_type = req.headers.iter().find(|(k, _)| k == "Content-Type").map(|(_, v)| v.clone()).unwrap();
        let boundary = content_type.strip_prefix("multipart/form-data; boundary=").expect("boundary in content type");
        let body = String::from_utf8_lossy(req.body.as_deref().unwrap()).into_owned();
        let caption = body.find("name=\"caption\"").expect("text field");
        let image = body.find("name=\"image\"; filename=\"sun.png\"").expect("file part");
        assert!(caption < image);
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn uploads_are_rejected_for_get() {
        let mut r = ServiceRequest::new(RequestMethod::Get, "https://example.test/feed", &[]);
        r.add_multipart_data(b"x", "f", "", None);
        assert_eq!(r.prepared_url_request().unwrap_err(), "multipart data needs POST or PUT");
    }

    #[test]
    fn post_without_parameters_has_no_body() {
        let req = ServiceRequest::new(RequestMethod::Post, "https://example.test/ping", &[]).prepared_url_request().unwrap();
        assert_eq!(req.method, "POST");
        assert!(req.body.is_none());
        assert!(req.headers.is_empty());
    }

    #[test]
    fn method_strings_and_bodies_match_the_request_methods() {
        assert_eq!(RequestMethod::Get.as_str(), "GET");
        assert_eq!(RequestMethod::Put.as_str(), "PUT");
        assert!(RequestMethod::Post.has_body());
        assert!(!RequestMethod::Delete.has_body());
    }
}

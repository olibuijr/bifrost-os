//! `NetService` and `NetServiceBrowser` (Bonjour) parity over `dns_service`.
//!
//! `NetService` is a published service: a name, a type such as `_http._tcp`, a port and TXT data, with the
//! `publish()` / `stop()` state. `NetServiceBrowser` keeps the set of services found for a type and reports
//! `didFind` / `didRemove` to its delegate. No multicast packets are sent here: a transport (the shell's mDNS
//! responder) calls `service_appeared` / `service_disappeared` and reads `published()` to answer queries.

use crate::dns_service::{ServiceError, ServiceInstance, ServiceType};
use std::sync::{Arc, Mutex};

/// Where a `NetService` is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetServiceState {
    Idle,
    Published,
    Stopped,
}

/// `NetService`.
#[derive(Clone, Debug)]
pub struct NetService {
    instance: ServiceInstance,
    state: NetServiceState,
}

impl NetService {
    /// `init(domain:type:name:port:)`; the domain is always `local.`.
    pub fn new(name: &str, service_type: &str, port: u16) -> Result<Self, ServiceError> {
        let ty = ServiceType::parse(service_type)?;
        Ok(NetService { instance: ServiceInstance::new(name, ty, port)?, state: NetServiceState::Idle })
    }
    /// `setTXTRecord(_:)` from key/value pairs.
    pub fn set_txt_record(&mut self, pairs: &[(&str, &str)]) -> Result<(), ServiceError> {
        self.instance.set_txt(pairs)
    }
    pub fn name(&self) -> &str {
        self.instance.name()
    }
    pub fn port(&self) -> u16 {
        self.instance.port()
    }
    pub fn domain(&self) -> &str {
        self.instance.domain()
    }
    pub fn service_type(&self) -> String {
        self.instance.service().to_string()
    }
    pub fn full_name(&self) -> String {
        self.instance.fqdn()
    }
    pub fn txt(&self) -> &[(String, Option<String>)] {
        self.instance.txt()
    }
    pub fn state(&self) -> NetServiceState {
        self.state
    }
    /// `publish()`.
    pub fn publish(&mut self) {
        self.state = NetServiceState::Published;
    }
    /// `stop()`.
    pub fn stop(&mut self) {
        if self.state == NetServiceState::Published {
            self.state = NetServiceState::Stopped;
        }
    }
    pub fn instance(&self) -> &ServiceInstance {
        &self.instance
    }
}

/// `NetServiceBrowserDelegate`.
pub trait NetServiceBrowserDelegate: Send {
    fn did_find(&mut self, service: &NetService, more_coming: bool);
    fn did_remove(&mut self, service: &NetService, more_coming: bool);
}

/// `NetServiceBrowser`.
pub struct NetServiceBrowser {
    searching_for: Option<ServiceType>,
    found: Vec<NetService>,
    delegate: Option<Box<dyn NetServiceBrowserDelegate>>,
}

impl NetServiceBrowser {
    pub fn new() -> Self {
        NetServiceBrowser { searching_for: None, found: Vec::new(), delegate: None }
    }
    pub fn set_delegate(&mut self, delegate: Box<dyn NetServiceBrowserDelegate>) {
        self.delegate = Some(delegate);
    }
    /// `searchForServices(ofType:inDomain:)`.
    pub fn search_for_services(&mut self, service_type: &str) -> Result<(), ServiceError> {
        self.searching_for = Some(ServiceType::parse(service_type)?);
        self.found.clear();
        Ok(())
    }
    /// `stop()`: stops the search and forgets what was found.
    pub fn stop(&mut self) {
        self.searching_for = None;
        self.found.clear();
    }
    pub fn found(&self) -> &[NetService] {
        &self.found
    }
    /// Called by the transport when an announcement arrives. Other types and repeated names are ignored.
    pub fn service_appeared(&mut self, service: NetService, more_coming: bool) {
        let wanted = matches!(&self.searching_for, Some(t) if *t == *service.instance.service());
        if !wanted || self.found.iter().any(|s| s.full_name() == service.full_name()) {
            return;
        }
        if let Some(d) = self.delegate.as_mut() {
            d.did_find(&service, more_coming);
        }
        self.found.push(service);
    }
    /// Called by the transport when a goodbye arrives or a record expires.
    pub fn service_disappeared(&mut self, full_name: &str, more_coming: bool) {
        if let Some(i) = self.found.iter().position(|s| s.full_name() == full_name) {
            let gone = self.found.remove(i);
            if let Some(d) = self.delegate.as_mut() {
                d.did_remove(&gone, more_coming);
            }
        }
    }
}

impl Default for NetServiceBrowser {
    fn default() -> Self {
        Self::new()
    }
}

/// A delegate that records events, for tests and simple apps.
#[derive(Clone, Default)]
pub struct RecordingBrowserDelegate {
    pub events: Arc<Mutex<Vec<String>>>,
}

impl NetServiceBrowserDelegate for RecordingBrowserDelegate {
    fn did_find(&mut self, service: &NetService, _more_coming: bool) {
        self.events.lock().unwrap().push(format!("find {}", service.name()));
    }
    fn did_remove(&mut self, service: &NetService, _more_coming: bool) {
        self.events.lock().unwrap().push(format!("remove {}", service.name()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_publishes_and_stops() {
        let mut s = NetService::new("Office", "_http._tcp", 8080).unwrap();
        assert_eq!(s.state(), NetServiceState::Idle);
        s.set_txt_record(&[("path", "/")]).unwrap();
        s.publish();
        assert_eq!(s.state(), NetServiceState::Published);
        assert_eq!(s.full_name(), "Office._http._tcp.local.");
        s.stop();
        assert_eq!(s.state(), NetServiceState::Stopped);
        assert!(NetService::new("x", "http", 1).is_err());
    }

    #[test]
    fn a_browser_reports_services_of_its_type_once() {
        let rec = RecordingBrowserDelegate::default();
        let mut b = NetServiceBrowser::new();
        b.set_delegate(Box::new(rec.clone()));
        b.search_for_services("_http._tcp").unwrap();
        b.service_appeared(NetService::new("A", "_http._tcp", 1).unwrap(), false);
        b.service_appeared(NetService::new("A", "_http._tcp", 1).unwrap(), false);
        b.service_appeared(NetService::new("P", "_ipp._tcp", 1).unwrap(), false);
        assert_eq!(b.found().len(), 1);
        b.service_disappeared("A._http._tcp.local.", false);
        assert!(b.found().is_empty());
        assert_eq!(*rec.events.lock().unwrap(), vec!["find A".to_string(), "remove A".to_string()]);
    }
}

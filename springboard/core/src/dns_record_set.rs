//! Resource record sets for one DNS-SD service instance (std only).
//!
//! A record set holds the records a service publishes. Each record gets a handle when it is
//! added, so it can be changed or removed later, the way the add, update and remove calls work
//! on a registered service. `service_records` builds the four records that RFC 6763 gives a
//! service instance: a PTR from the service type to the instance, SRV and TXT for the instance,
//! and an A record for the host. The set sends nothing: the service layer encodes it with
//! `dns_message` and sends the bytes.

use std::fmt;

use crate::dns_message::{self, DnsError, Message, RData, Record, CLASS_IN};

/// Time to live of the shared PTR record, in seconds.
pub const PTR_TTL: u32 = 4500;
/// Time to live of the host-owned SRV and A records, in seconds.
pub const HOST_TTL: u32 = 120;
/// Longest instance label, in bytes.
const MAX_INSTANCE: usize = 63;

/// Handle for one record in a set. Handles are never reused within one set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecordId(u32);

/// Why a change to a set was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordSetError {
    /// No record in the set has this handle, either because it was never added or was removed.
    NotFound,
}

impl fmt::Display for RecordSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no record with that handle is in the set")
    }
}

impl std::error::Error for RecordSetError {}

/// The records a service publishes, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordSet {
    next: u32,
    entries: Vec<(RecordId, Record)>,
}

impl RecordSet {
    /// An empty set.
    pub fn new() -> RecordSet {
        RecordSet::default()
    }

    /// Adds a record and returns its handle.
    pub fn add(&mut self, record: Record) -> RecordId {
        let id = RecordId(self.next);
        self.next = self.next.wrapping_add(1);
        self.entries.push((id, record));
        id
    }

    /// Replaces the time to live and the data of the record with this handle.
    /// The name, class and cache-flush bit stay as they were.
    pub fn update(&mut self, id: RecordId, ttl: u32, data: RData) -> Result<(), RecordSetError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.0 == id)
            .ok_or(RecordSetError::NotFound)?;
        entry.1.ttl = ttl;
        entry.1.data = data;
        Ok(())
    }

    /// Removes the record with this handle and returns it.
    pub fn remove(&mut self, id: RecordId) -> Result<Record, RecordSetError> {
        let at = self
            .entries
            .iter()
            .position(|e| e.0 == id)
            .ok_or(RecordSetError::NotFound)?;
        Ok(self.entries.remove(at).1)
    }

    /// The records in the order they were added.
    pub fn records(&self) -> Vec<&Record> {
        self.entries.iter().map(|(_, r)| r).collect()
    }

    /// Number of records in the set.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the set holds no records.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The whole set as an authoritative response, with every record in the answer section.
    pub fn as_response(&self, id: u16) -> Message {
        Message {
            id,
            response: true,
            authoritative: true,
            questions: Vec::new(),
            answers: self.entries.iter().map(|(_, r)| r.clone()).collect(),
            authorities: Vec::new(),
            additionals: Vec::new(),
        }
    }
}

/// The names and data of one service instance, such as an office printer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceInstance<'a> {
    /// The instance label, such as "Office printer". It may not contain a dot.
    pub instance: &'a str,
    /// The service type and transport, such as "_ipp._tcp".
    pub service_type: &'a str,
    /// The domain, such as "local".
    pub domain: &'a str,
    /// The host name that the SRV and A records describe, such as "box.local".
    pub host: &'a str,
    /// The port the service listens on.
    pub port: u16,
    /// The TXT strings, each a length byte followed by its bytes.
    pub txt: &'a [u8],
    /// The IPv4 address of the host.
    pub address: [u8; 4],
}

/// Builds the records for one service instance:
/// a PTR from `service_type.domain` to the instance, an SRV and a TXT record for the instance,
/// and an A record for the host. Names and TXT data are checked before anything is built.
pub fn service_records(service: &ServiceInstance<'_>) -> Result<RecordSet, DnsError> {
    if service.instance.is_empty()
        || service.instance.len() > MAX_INSTANCE
        || service.instance.contains('.')
    {
        return Err(DnsError::BadName);
    }
    let ptr_name = format!("{}.{}", service.service_type, service.domain);
    let instance_name = format!("{}.{}", service.instance, ptr_name);
    check_name(&ptr_name)?;
    check_name(&instance_name)?;
    check_name(service.host)?;
    if !dns_message::txt_is_valid(service.txt) {
        return Err(DnsError::BadLength);
    }

    let mut set = RecordSet::new();
    set.add(Record {
        name: ptr_name,
        class: CLASS_IN,
        cache_flush: false,
        ttl: PTR_TTL,
        data: RData::Ptr(instance_name.clone()),
    });
    set.add(Record {
        name: instance_name.clone(),
        class: CLASS_IN,
        cache_flush: true,
        ttl: HOST_TTL,
        data: RData::Srv {
            priority: 0,
            weight: 0,
            port: service.port,
            target: service.host.to_string(),
        },
    });
    set.add(Record {
        name: instance_name,
        class: CLASS_IN,
        cache_flush: true,
        ttl: PTR_TTL,
        data: RData::Txt(service.txt.to_vec()),
    });
    set.add(Record {
        name: service.host.to_string(),
        class: CLASS_IN,
        cache_flush: true,
        ttl: HOST_TTL,
        data: RData::A(service.address),
    });
    Ok(set)
}

/// Fails when `name` cannot be written as DNS labels.
fn check_name(name: &str) -> Result<(), DnsError> {
    let mut scratch = Vec::new();
    dns_message::encode_name(name, &mut scratch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer<'a>(txt: &'a [u8]) -> ServiceInstance<'a> {
        ServiceInstance {
            instance: "Printer",
            service_type: "_ipp._tcp",
            domain: "local",
            host: "box.local",
            port: 631,
            txt,
            address: [192, 168, 1, 20],
        }
    }

    fn a_record(name: &str, addr: [u8; 4]) -> Record {
        Record {
            name: name.to_string(),
            class: CLASS_IN,
            cache_flush: true,
            ttl: HOST_TTL,
            data: RData::A(addr),
        }
    }

    #[test]
    fn service_builds_ptr_srv_txt_and_a_in_that_order() {
        let txt = [3u8, b'a', b'=', b'1'];
        let set = service_records(&printer(&txt)).unwrap();
        assert_eq!(set.len(), 4);
        let records = set.records();
        assert_eq!(records[0].name, "_ipp._tcp.local");
        assert_eq!(records[0].data, RData::Ptr("Printer._ipp._tcp.local".to_string()));
        assert!(!records[0].cache_flush);
        assert_eq!(records[0].ttl, PTR_TTL);
        assert_eq!(records[1].name, "Printer._ipp._tcp.local");
        assert_eq!(
            records[1].data,
            RData::Srv {
                priority: 0,
                weight: 0,
                port: 631,
                target: "box.local".to_string(),
            }
        );
        assert_eq!(records[2].data, RData::Txt(vec![3, b'a', b'=', b'1']));
        assert_eq!(records[3].name, "box.local");
        assert_eq!(records[3].data, RData::A([192, 168, 1, 20]));
        assert_eq!(records[3].ttl, HOST_TTL);
    }

    #[test]
    fn set_encodes_as_a_response_and_decodes_to_the_same_records() {
        let txt = [3u8, b'a', b'=', b'1'];
        let set = service_records(&printer(&txt)).unwrap();
        let bytes = dns_message::encode(&set.as_response(0x0102)).unwrap();
        // Id, then flags 0x8400 (response, authoritative), then no questions and four answers.
        assert_eq!(&bytes[..8], &[0x01, 0x02, 0x84, 0x00, 0, 0, 0, 4]);
        let back = dns_message::decode(&bytes).unwrap();
        assert!(back.response && back.authoritative);
        let expected: Vec<Record> = set.records().into_iter().cloned().collect();
        assert_eq!(back.answers, expected);
    }

    #[test]
    fn added_records_can_be_updated_and_removed_by_handle() {
        let mut set = RecordSet::new();
        let first = set.add(a_record("box.local", [10, 0, 0, 1]));
        let second = set.add(a_record("nas.local", [10, 0, 0, 2]));
        assert_ne!(first, second);
        assert_eq!(set.len(), 2);

        set.update(first, 60, RData::A([10, 0, 0, 5])).unwrap();
        let records = set.records();
        assert_eq!(records[0].ttl, 60);
        assert_eq!(records[0].data, RData::A([10, 0, 0, 5]));
        assert_eq!(records[0].name, "box.local");
        assert_eq!(records[1].data, RData::A([10, 0, 0, 2]));

        let removed = set.remove(first).unwrap();
        assert_eq!(removed.data, RData::A([10, 0, 0, 5]));
        assert_eq!(set.len(), 1);
        assert_eq!(set.records()[0].name, "nas.local");
    }

    #[test]
    fn unknown_or_removed_handles_are_not_found() {
        let mut set = RecordSet::new();
        let id = set.add(a_record("box.local", [10, 0, 0, 1]));
        set.remove(id).unwrap();
        assert_eq!(set.remove(id), Err(RecordSetError::NotFound));
        assert_eq!(
            set.update(id, 10, RData::A([1, 1, 1, 1])),
            Err(RecordSetError::NotFound)
        );
        assert!(set.is_empty());
    }

    #[test]
    fn instance_with_a_dot_or_a_long_label_is_refused() {
        let txt = [0u8; 0];
        let mut dotted = printer(&txt);
        dotted.instance = "Mr. Smith";
        assert_eq!(service_records(&dotted), Err(DnsError::BadName));

        let long = "a".repeat(64);
        let mut too_long = printer(&txt);
        too_long.instance = long.as_str();
        assert_eq!(service_records(&too_long), Err(DnsError::BadName));
    }

    #[test]
    fn txt_that_is_not_length_prefixed_is_refused() {
        // The first string claims three bytes but only one follows.
        let bad = [3u8, b'a'];
        assert_eq!(service_records(&printer(&bad)), Err(DnsError::BadLength));
    }
}

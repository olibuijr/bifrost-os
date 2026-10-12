//! Network services, network sets (locations) and the service order inside each set.
//!
//! A service is one network connection the system knows: a wired link or a Wi-Fi network, with the interface it uses.
//! A set is a named location, such as home or office. Each set keeps an ordered list of services, and the first
//! enabled service in that order carries traffic first.
//!
//! This is a pure model. It opens no interface, joins no network and writes no settings file. The caller keeps a
//! [`NetworkLocations`], saves it in its own settings and reads [`NetworkLocations::active_services`] to decide which
//! connection to use.
//!
//! Rules of this model:
//! - Service and set ids are non-empty and unique among their kind.
//! - [`NetworkLocations::add_service`] appends the new service to the end of every set's order.
//! - A set's order may name a subset of the services. Services left out of a set's order are not used in that set.
//! - The current set cannot be removed. Removing a service also removes it from every order.

use std::fmt;

/// One network connection: an id, a name shown to people, the interface it uses and whether it is switched on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkService {
    pub id: String,
    pub name: String,
    pub interface: String,
    pub enabled: bool,
}

/// A named location with an ordered list of service ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSet {
    pub id: String,
    pub name: String,
    pub service_order: Vec<String>,
}

/// Why a change is refused. A refused change leaves the locations as they were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkError {
    EmptyId,
    DuplicateService(String),
    DuplicateSet(String),
    UnknownService(String),
    UnknownSet(String),
    DuplicateInOrder(String),
    CannotRemoveCurrentSet,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkError::EmptyId => write!(f, "the id is empty"),
            NetworkError::DuplicateService(id) => write!(f, "a service with the id {id:?} already exists"),
            NetworkError::DuplicateSet(id) => write!(f, "a set with the id {id:?} already exists"),
            NetworkError::UnknownService(id) => write!(f, "there is no service with the id {id:?}"),
            NetworkError::UnknownSet(id) => write!(f, "there is no set with the id {id:?}"),
            NetworkError::DuplicateInOrder(id) => write!(f, "the service {id:?} is listed twice in the order"),
            NetworkError::CannotRemoveCurrentSet => write!(f, "the current set cannot be removed"),
        }
    }
}

/// The services, the sets and the id of the set in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkLocations {
    services: Vec<NetworkService>,
    sets: Vec<NetworkSet>,
    current: String,
}

fn check_id(id: &str) -> Result<(), NetworkError> {
    if id.trim().is_empty() {
        Err(NetworkError::EmptyId)
    } else {
        Ok(())
    }
}

impl NetworkLocations {
    /// A model with one set, `id` and `name`, that is the current set. It holds no services yet.
    pub fn new(set_id: &str, set_name: &str) -> Result<NetworkLocations, NetworkError> {
        check_id(set_id)?;
        let set = NetworkSet { id: set_id.to_string(), name: set_name.to_string(), service_order: Vec::new() };
        Ok(NetworkLocations { services: Vec::new(), sets: vec![set], current: set_id.to_string() })
    }

    pub fn services(&self) -> &[NetworkService] {
        &self.services
    }

    pub fn sets(&self) -> &[NetworkSet] {
        &self.sets
    }

    pub fn current_set_id(&self) -> &str {
        &self.current
    }

    fn set_index(&self, id: &str) -> Option<usize> {
        self.sets.iter().position(|s| s.id == id)
    }

    /// Adds a service, enabled, and appends it to the order of every set.
    pub fn add_service(&mut self, id: &str, name: &str, interface: &str) -> Result<(), NetworkError> {
        check_id(id)?;
        if self.services.iter().any(|s| s.id == id) {
            return Err(NetworkError::DuplicateService(id.to_string()));
        }
        self.services.push(NetworkService {
            id: id.to_string(),
            name: name.to_string(),
            interface: interface.to_string(),
            enabled: true,
        });
        for set in &mut self.sets {
            set.service_order.push(id.to_string());
        }
        Ok(())
    }

    /// Removes a service from the list and from every set's order.
    pub fn remove_service(&mut self, id: &str) -> Result<(), NetworkError> {
        let before = self.services.len();
        self.services.retain(|s| s.id != id);
        if self.services.len() == before {
            return Err(NetworkError::UnknownService(id.to_string()));
        }
        for set in &mut self.sets {
            set.service_order.retain(|s| s != id);
        }
        Ok(())
    }

    /// Switches a service on or off. An off service stays in the order but is skipped by [`Self::active_services`].
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), NetworkError> {
        match self.services.iter_mut().find(|s| s.id == id) {
            Some(service) => {
                service.enabled = enabled;
                Ok(())
            }
            None => Err(NetworkError::UnknownService(id.to_string())),
        }
    }

    /// Adds a set. Its order starts empty; the caller fills it with [`Self::set_service_order`].
    pub fn add_set(&mut self, id: &str, name: &str) -> Result<(), NetworkError> {
        check_id(id)?;
        if self.set_index(id).is_some() {
            return Err(NetworkError::DuplicateSet(id.to_string()));
        }
        self.sets.push(NetworkSet { id: id.to_string(), name: name.to_string(), service_order: Vec::new() });
        Ok(())
    }

    /// Makes a set the current one.
    pub fn set_current(&mut self, id: &str) -> Result<(), NetworkError> {
        if self.set_index(id).is_none() {
            return Err(NetworkError::UnknownSet(id.to_string()));
        }
        self.current = id.to_string();
        Ok(())
    }

    /// Replaces the order of a set. Every id must name an existing service and appear once.
    pub fn set_service_order(&mut self, set_id: &str, order: &[&str]) -> Result<(), NetworkError> {
        let index = self.set_index(set_id).ok_or_else(|| NetworkError::UnknownSet(set_id.to_string()))?;
        let mut seen: Vec<&str> = Vec::with_capacity(order.len());
        for id in order {
            if !self.services.iter().any(|s| s.id == *id) {
                return Err(NetworkError::UnknownService((*id).to_string()));
            }
            if seen.contains(id) {
                return Err(NetworkError::DuplicateInOrder((*id).to_string()));
            }
            seen.push(*id);
        }
        self.sets[index].service_order = order.iter().map(|s| s.to_string()).collect();
        Ok(())
    }

    /// Removes a set. The current set cannot be removed; switch to another set first.
    pub fn remove_set(&mut self, id: &str) -> Result<(), NetworkError> {
        let index = self.set_index(id).ok_or_else(|| NetworkError::UnknownSet(id.to_string()))?;
        if self.current == id {
            return Err(NetworkError::CannotRemoveCurrentSet);
        }
        self.sets.remove(index);
        Ok(())
    }

    /// The enabled services of the current set, in order. Ids in the order that name no service are skipped.
    pub fn active_services(&self) -> Vec<&NetworkService> {
        let Some(index) = self.set_index(&self.current) else {
            return Vec::new();
        };
        self.sets[index]
            .service_order
            .iter()
            .filter_map(|id| self.services.iter().find(|s| &s.id == id))
            .filter(|s| s.enabled)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_services() -> NetworkLocations {
        let mut net = NetworkLocations::new("home", "Home").unwrap();
        net.add_service("wired", "Ethernet", "eth0").unwrap();
        net.add_service("wifi", "Wi-Fi", "wlan0").unwrap();
        net
    }

    fn ids(list: &[&NetworkService]) -> Vec<String> {
        list.iter().map(|s| s.id.clone()).collect()
    }

    #[test]
    fn added_services_follow_the_order_of_the_current_set() {
        let net = two_services();
        assert_eq!(ids(&net.active_services()), vec!["wired", "wifi"]);
        assert_eq!(net.sets()[0].service_order, vec!["wired", "wifi"]);
    }

    #[test]
    fn the_order_can_be_replaced_and_disabled_services_are_skipped() {
        let mut net = two_services();
        net.set_service_order("home", &["wifi", "wired"]).unwrap();
        assert_eq!(ids(&net.active_services()), vec!["wifi", "wired"]);
        net.set_enabled("wifi", false).unwrap();
        assert_eq!(ids(&net.active_services()), vec!["wired"]);
    }

    #[test]
    fn a_bad_order_is_refused_and_leaves_the_old_order() {
        let mut net = two_services();
        assert_eq!(
            net.set_service_order("home", &["wifi", "dialup"]),
            Err(NetworkError::UnknownService("dialup".to_string()))
        );
        assert_eq!(
            net.set_service_order("home", &["wifi", "wifi"]),
            Err(NetworkError::DuplicateInOrder("wifi".to_string()))
        );
        assert_eq!(net.sets()[0].service_order, vec!["wired", "wifi"]);
    }

    #[test]
    fn switching_sets_changes_the_services_in_use() {
        let mut net = two_services();
        net.add_set("office", "Office").unwrap();
        assert!(net.active_services().len() == 2);
        net.set_service_order("office", &["wifi"]).unwrap();
        net.set_current("office").unwrap();
        assert_eq!(net.current_set_id(), "office");
        assert_eq!(ids(&net.active_services()), vec!["wifi"]);
        assert_eq!(net.set_current("cabin"), Err(NetworkError::UnknownSet("cabin".to_string())));
    }

    #[test]
    fn the_current_set_cannot_be_removed_but_another_can() {
        let mut net = two_services();
        net.add_set("office", "Office").unwrap();
        assert_eq!(net.remove_set("home"), Err(NetworkError::CannotRemoveCurrentSet));
        net.remove_set("office").unwrap();
        assert_eq!(net.sets().len(), 1);
    }

    #[test]
    fn removing_a_service_removes_it_from_every_order() {
        let mut net = two_services();
        net.add_set("office", "Office").unwrap();
        net.set_service_order("office", &["wired", "wifi"]).unwrap();
        net.remove_service("wifi").unwrap();
        assert!(net.services().iter().all(|s| s.id != "wifi"));
        assert!(net.sets().iter().all(|s| !s.service_order.contains(&"wifi".to_string())));
        assert_eq!(net.remove_service("wifi"), Err(NetworkError::UnknownService("wifi".to_string())));
    }

    #[test]
    fn duplicate_and_empty_ids_are_refused() {
        let mut net = two_services();
        assert_eq!(
            net.add_service("wired", "Again", "eth1"),
            Err(NetworkError::DuplicateService("wired".to_string()))
        );
        assert_eq!(net.add_set("home", "Again"), Err(NetworkError::DuplicateSet("home".to_string())));
        assert_eq!(net.add_service("  ", "Blank", "eth2"), Err(NetworkError::EmptyId));
        assert_eq!(NetworkLocations::new("", "None").unwrap_err(), NetworkError::EmptyId);
    }
}

//! The object tree: System, Plugin, Device, Stream, Control (Volume, Mute, DataSource, Selector), Box, Clock, Tap and
//! Aggregate objects with typed properties addressed by (selector, scope, element), get, set, and listeners.
//!
//! Listener registration is per (object, address). Changes are not delivered at once: they collect in a pending
//! set (deduplicated) and `flush` hands out one coalesced notice per listening client, once per main-loop turn.
//!
//! Owner: task 02. Log category: `is.olibuijr.audio` / `client` (the server loop logs; this module is pure).

use bifrost_audio_types::address::{Class, ObjectId, PropertyAddress, SYSTEM_OBJECT};
use bifrost_audio_types::error::AudioError;
use bifrost_audio_types::props::{self, scope_bit};
use bifrost_audio_types::value::PropertyValue;
use std::collections::{BTreeMap, HashMap};

/// A client id as the server hands out in `Welcome`.
pub type ClientId = u32;

/// One coalesced change notice for a client: re-read these addresses.
pub type Notice = (ClientId, Vec<(ObjectId, PropertyAddress)>);

#[derive(Debug)]
struct Object {
    class: Class,
    owner: Option<ObjectId>,
    owned: Vec<ObjectId>,
    props: HashMap<PropertyAddress, PropertyValue>,
}

/// The tree.
#[derive(Debug)]
pub struct ObjectTree {
    objects: BTreeMap<u32, Object>,
    next_id: u32,
    listeners: HashMap<(ObjectId, PropertyAddress), Vec<ClientId>>,
    pending: Vec<(ObjectId, PropertyAddress)>,
}

impl Default for ObjectTree {
    fn default() -> Self {
        ObjectTree::new()
    }
}

impl ObjectTree {
    /// A tree holding only the system object (id 1).
    pub fn new() -> ObjectTree {
        let mut objects = BTreeMap::new();
        objects.insert(
            SYSTEM_OBJECT.0,
            Object { class: Class::System, owner: None, owned: Vec::new(), props: HashMap::new() },
        );
        ObjectTree { objects, next_id: 2, listeners: HashMap::new(), pending: Vec::new() }
    }

    pub fn class(&self, id: ObjectId) -> Result<Class, AudioError> {
        self.objects.get(&id.0).map(|o| o.class).ok_or(AudioError::UnknownObject)
    }

    pub fn owner(&self, id: ObjectId) -> Result<Option<ObjectId>, AudioError> {
        self.objects.get(&id.0).map(|o| o.owner).ok_or(AudioError::UnknownObject)
    }

    /// Adds an object of `class` owned by `owner`; ids are never reused.
    pub fn add(&mut self, class: Class, owner: ObjectId) -> Result<ObjectId, AudioError> {
        if !self.objects.contains_key(&owner.0) {
            return Err(AudioError::UnknownObject);
        }
        while self.objects.contains_key(&self.next_id) {
            self.next_id += 1;
        }
        let id = ObjectId(self.next_id);
        self.next_id += 1;
        self.objects.insert(id.0, Object { class, owner: Some(owner), owned: Vec::new(), props: HashMap::new() });
        if let Some(o) = self.objects.get_mut(&owner.0) {
            o.owned.push(id);
        }
        Ok(id)
    }

    /// Adds an object of `class` owned by `owner` under an id the caller chose (device objects keep the ids the control
    /// side gave them). Refused when the id is 0 or taken; `add` skips ids taken this way.
    pub fn add_with_id(&mut self, class: Class, owner: ObjectId, id: ObjectId) -> Result<ObjectId, AudioError> {
        if id.0 == 0 || self.objects.contains_key(&id.0) {
            return Err(AudioError::BadValue);
        }
        if !self.objects.contains_key(&owner.0) {
            return Err(AudioError::UnknownObject);
        }
        self.objects.insert(id.0, Object { class, owner: Some(owner), owned: Vec::new(), props: HashMap::new() });
        if let Some(o) = self.objects.get_mut(&owner.0) {
            o.owned.push(id);
        }
        Ok(id)
    }

    /// Removes an object and everything it owns, with their listeners. Returns the removed ids (the object first).
    /// The system object cannot be removed.
    pub fn remove(&mut self, id: ObjectId) -> Result<Vec<ObjectId>, AudioError> {
        if id == SYSTEM_OBJECT {
            return Err(AudioError::BadValue);
        }
        let owner = self.owner(id)?;
        let mut gone = vec![id];
        let mut i = 0;
        while i < gone.len() {
            let kids = self.objects[&gone[i].0].owned.clone();
            gone.extend(kids);
            i += 1;
        }
        for g in &gone {
            self.objects.remove(&g.0);
        }
        self.listeners.retain(|(o, _), _| !gone.contains(o));
        self.pending.retain(|(o, _)| !gone.contains(o));
        if let Some(Some(o)) = owner.map(|o| self.objects.get_mut(&o.0)) {
            o.owned.retain(|c| *c != id);
        }
        Ok(gone)
    }

    /// The objects owned by `id`, in creation order.
    pub fn owned(&self, id: ObjectId) -> Result<Vec<ObjectId>, AudioError> {
        self.objects.get(&id.0).map(|o| o.owned.clone()).ok_or(AudioError::UnknownObject)
    }

    /// Gives an object a property (at server setup, or when hardware appears). The selector must be in the table,
    /// carried by the object's class, valid in the address's scope, with a value of the right type.
    pub fn define(&mut self, id: ObjectId, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        let class = self.class(id)?;
        let def = props::lookup(address.selector).ok_or(AudioError::UnknownProperty)?;
        if !def.classes.contains(&class) || def.scopes & scope_bit(address.scope) == 0 {
            return Err(AudioError::UnknownProperty);
        }
        props::check_type(def, &value)?;
        let obj = self.objects.get_mut(&id.0).ok_or(AudioError::UnknownObject)?;
        let changed = obj.props.get(&address) != Some(&value);
        obj.props.insert(address, value);
        if changed {
            self.mark(id, address);
        }
        Ok(())
    }

    /// The server's own update of a property (settable or not): same checks as `set` except the settable flag.
    pub fn update(&mut self, id: ObjectId, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        let class = self.class(id)?;
        let def = props::lookup(address.selector).ok_or(AudioError::UnknownProperty)?;
        let obj = self.objects.get_mut(&id.0).ok_or(AudioError::UnknownObject)?;
        if !def.classes.contains(&class) || !obj.props.contains_key(&address) {
            return Err(AudioError::UnknownProperty);
        }
        props::check_type(def, &value)?;
        if obj.props.get(&address) != Some(&value) {
            obj.props.insert(address, value);
            self.mark(id, address);
        }
        Ok(())
    }

    pub fn get(&self, id: ObjectId, address: PropertyAddress) -> Result<PropertyValue, AudioError> {
        let obj = self.objects.get(&id.0).ok_or(AudioError::UnknownObject)?;
        props::lookup(address.selector).ok_or(AudioError::UnknownProperty)?;
        obj.props.get(&address).cloned().ok_or(AudioError::UnknownProperty)
    }

    /// A client's set: the property must exist on the object, be settable and have the right type. Range and
    /// ownership checks (rates, buffer sizes, hog) belong to the device layer that calls this.
    pub fn set(&mut self, id: ObjectId, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        let def = props::lookup(address.selector).ok_or(AudioError::UnknownProperty)?;
        if !self.objects.get(&id.0).ok_or(AudioError::UnknownObject)?.props.contains_key(&address) {
            return Err(AudioError::UnknownProperty);
        }
        if !def.settable {
            return Err(AudioError::NotSettable);
        }
        self.update(id, address, value)
    }

    /// Registers `client` for changes of (object, address). The property must exist. Idempotent.
    pub fn listen(&mut self, client: ClientId, id: ObjectId, address: PropertyAddress) -> Result<(), AudioError> {
        self.get(id, address)?;
        let l = self.listeners.entry((id, address)).or_default();
        if !l.contains(&client) {
            l.push(client);
        }
        Ok(())
    }

    pub fn unlisten(&mut self, client: ClientId, id: ObjectId, address: PropertyAddress) -> Result<(), AudioError> {
        self.get(id, address)?;
        if let Some(l) = self.listeners.get_mut(&(id, address)) {
            l.retain(|c| *c != client);
            if l.is_empty() {
                self.listeners.remove(&(id, address));
            }
        }
        Ok(())
    }

    /// Drops every listener of a client that went away.
    pub fn drop_client(&mut self, client: ClientId) {
        for l in self.listeners.values_mut() {
            l.retain(|c| *c != client);
        }
        self.listeners.retain(|_, l| !l.is_empty());
    }

    fn mark(&mut self, id: ObjectId, address: PropertyAddress) {
        if self.listeners.contains_key(&(id, address)) && !self.pending.contains(&(id, address)) {
            self.pending.push((id, address));
        }
    }

    /// Ends a main-loop turn: one notice per client with its changed addresses (in change order), clients in id
    /// order. Many changes of one address in the turn give one entry.
    pub fn flush(&mut self) -> Vec<Notice> {
        let mut per: BTreeMap<ClientId, Vec<(ObjectId, PropertyAddress)>> = BTreeMap::new();
        for key in self.pending.drain(..) {
            if let Some(cs) = self.listeners.get(&key) {
                for c in cs {
                    per.entry(*c).or_default().push(key);
                }
            }
        }
        per.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::address::Scope;

    fn g(sel: bifrost_audio_types::FourCC) -> PropertyAddress {
        PropertyAddress::global(sel)
    }

    fn device_tree() -> (ObjectTree, ObjectId) {
        let mut t = ObjectTree::new();
        let d = t.add(Class::Device, SYSTEM_OBJECT).unwrap();
        t.define(d, g(props::NOMINAL_RATE), PropertyValue::F64(48000.0)).unwrap();
        t.define(d, g(props::IS_ALIVE), PropertyValue::Bool(true)).unwrap();
        t.define(d, g(props::BUFFER_FRAMES), PropertyValue::U32(256)).unwrap();
        (t, d)
    }

    #[test]
    fn get_set_round_trip() {
        let (mut t, d) = device_tree();
        assert_eq!(t.get(d, g(props::NOMINAL_RATE)), Ok(PropertyValue::F64(48000.0)));
        t.set(d, g(props::NOMINAL_RATE), PropertyValue::F64(44100.0)).unwrap();
        assert_eq!(t.get(d, g(props::NOMINAL_RATE)), Ok(PropertyValue::F64(44100.0)));
    }

    #[test]
    fn read_only_set_is_refused_and_value_kept() {
        let (mut t, d) = device_tree();
        assert_eq!(t.set(d, g(props::IS_ALIVE), PropertyValue::Bool(false)), Err(AudioError::NotSettable));
        assert_eq!(t.get(d, g(props::IS_ALIVE)), Ok(PropertyValue::Bool(true)));
        // the server itself can still update it
        t.update(d, g(props::IS_ALIVE), PropertyValue::Bool(false)).unwrap();
        assert_eq!(t.get(d, g(props::IS_ALIVE)), Ok(PropertyValue::Bool(false)));
    }

    #[test]
    fn wrong_type_and_unknown_selector_and_object() {
        let (mut t, d) = device_tree();
        assert_eq!(t.set(d, g(props::BUFFER_FRAMES), PropertyValue::F64(64.0)), Err(AudioError::BadValueType));
        let zz = g(bifrost_audio_types::FourCC::new(b"zzzz"));
        assert_eq!(t.get(d, zz), Err(AudioError::UnknownProperty));
        assert_eq!(t.set(d, zz, PropertyValue::U32(1)), Err(AudioError::UnknownProperty));
        assert_eq!(t.listen(1, d, zz), Err(AudioError::UnknownProperty));
        // a known selector the object does not carry
        assert_eq!(t.get(d, g(props::HOG_PID)), Err(AudioError::UnknownProperty));
        assert_eq!(t.get(ObjectId(99), g(props::NAME)), Err(AudioError::UnknownObject));
        assert_eq!(t.set(ObjectId(99), g(props::NAME), PropertyValue::U32(0)), Err(AudioError::UnknownObject));
    }

    #[test]
    fn define_checks_class_and_scope() {
        let (mut t, d) = device_tree();
        // volume belongs to Volume controls, not devices
        assert_eq!(t.define(d, g(props::VOLUME_SCALAR), PropertyValue::F32(1.0)), Err(AudioError::UnknownProperty));
        let v = t.add(Class::Volume, d).unwrap();
        // volume exists in input and output scope, not global
        assert_eq!(t.define(v, g(props::VOLUME_SCALAR), PropertyValue::F32(1.0)), Err(AudioError::UnknownProperty));
        let out = PropertyAddress::scoped(props::VOLUME_SCALAR, Scope::Output);
        t.define(v, out, PropertyValue::F32(0.5)).unwrap();
        assert_eq!(t.get(v, out), Ok(PropertyValue::F32(0.5)));
    }

    #[test]
    fn listener_fires_once_per_coalesced_change() {
        let (mut t, d) = device_tree();
        let a = g(props::BUFFER_FRAMES);
        t.listen(7, d, a).unwrap();
        t.listen(7, d, a).unwrap(); // idempotent
        t.listen(8, d, a).unwrap();
        assert!(t.flush().is_empty());
        t.set(d, a, PropertyValue::U32(128)).unwrap();
        t.set(d, a, PropertyValue::U32(64)).unwrap();
        t.set(d, a, PropertyValue::U32(32)).unwrap();
        assert_eq!(t.flush(), vec![(7, vec![(d, a)]), (8, vec![(d, a)])]);
        // the next turn is quiet
        assert!(t.flush().is_empty());
        // setting the same value is not a change
        t.set(d, a, PropertyValue::U32(32)).unwrap();
        assert!(t.flush().is_empty());
    }

    #[test]
    fn only_listened_addresses_and_unlisten() {
        let (mut t, d) = device_tree();
        let a = g(props::BUFFER_FRAMES);
        let r = g(props::NOMINAL_RATE);
        t.listen(1, d, a).unwrap();
        t.set(d, r, PropertyValue::F64(96000.0)).unwrap();
        assert!(t.flush().is_empty());
        t.set(d, r, PropertyValue::F64(48000.0)).unwrap();
        t.set(d, a, PropertyValue::U32(64)).unwrap();
        t.unlisten(1, d, a).unwrap();
        // a pending notice for an address nobody listens to any more is dropped
        t.set(d, a, PropertyValue::U32(32)).unwrap();
        assert_eq!(t.flush(), vec![]);
    }

    #[test]
    fn drop_client_and_remove_clean_up() {
        let (mut t, d) = device_tree();
        let a = g(props::BUFFER_FRAMES);
        t.listen(1, d, a).unwrap();
        t.listen(2, d, a).unwrap();
        t.drop_client(1);
        t.set(d, a, PropertyValue::U32(64)).unwrap();
        assert_eq!(t.flush(), vec![(2, vec![(d, a)])]);
        let s = t.add(Class::Stream, d).unwrap();
        assert_eq!(t.owned(d), Ok(vec![s]));
        assert_eq!(t.remove(d), Ok(vec![d, s]));
        assert_eq!(t.class(s), Err(AudioError::UnknownObject));
        assert_eq!(t.owned(SYSTEM_OBJECT), Ok(vec![]));
        assert_eq!(t.remove(SYSTEM_OBJECT), Err(AudioError::BadValue));
    }

    #[test]
    fn chosen_ids_are_kept_and_never_handed_out_again() {
        let mut t = ObjectTree::new();
        assert_eq!(t.add_with_id(Class::Device, SYSTEM_OBJECT, ObjectId(2)), Ok(ObjectId(2)));
        assert_eq!(t.add_with_id(Class::Device, SYSTEM_OBJECT, ObjectId(2)), Err(AudioError::BadValue), "taken");
        assert_eq!(t.add_with_id(Class::Device, SYSTEM_OBJECT, ObjectId(0)), Err(AudioError::BadValue));
        assert_eq!(t.add_with_id(Class::Device, ObjectId(77), ObjectId(9)), Err(AudioError::UnknownObject));
        let next = t.add(Class::Device, SYSTEM_OBJECT).unwrap();
        assert_eq!(next, ObjectId(3), "add skips the taken id");
        assert_eq!(t.owned(SYSTEM_OBJECT), Ok(vec![ObjectId(2), ObjectId(3)]));
    }

    #[test]
    fn system_object_lists_devices() {
        let (mut t, d) = device_tree();
        let list = g(props::DEVICES);
        t.define(SYSTEM_OBJECT, list, PropertyValue::Objects(vec![])).unwrap();
        t.listen(5, SYSTEM_OBJECT, list).unwrap();
        t.update(SYSTEM_OBJECT, list, PropertyValue::Objects(vec![d])).unwrap();
        assert_eq!(t.flush(), vec![(5, vec![(SYSTEM_OBJECT, list)])]);
        assert_eq!(t.set(SYSTEM_OBJECT, list, PropertyValue::Objects(vec![])), Err(AudioError::NotSettable));
    }
}

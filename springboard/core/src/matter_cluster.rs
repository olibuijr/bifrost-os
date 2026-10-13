//! Matter cluster model: the clusters BifrOSt knows by name, their attributes and commands, and the checks that a
//! write or a command must pass before the transport sends it (std only).
//!
//! A cluster is a table of attributes and commands with fixed IDs. The table says which attributes are writable,
//! which fields a command takes, and the range each integer value must stay in. Typed state for the two common
//! clusters (On/Off and Level Control) follows the cluster rules: a move clamps to the minimum and maximum level,
//! and the On/Off commands change only the on/off state. The model sends nothing and encodes nothing: the caller
//! owns the transport and the wire format, and list-valued attributes stay opaque to this module.
//!
//! The IDs and field names follow the public cluster specification. The clusters here are a first set (Descriptor,
//! Identify, On/Off and Level Control); the rest of the cluster library is not modelled yet. Global attributes (such
//! as the attribute list and cluster revision) are not in the tables. The range rules, the required-field rule and
//! the error vocabulary are this module's own.

use crate::matter_interaction::{AttributePath, AttributeValue, CommandRequest, WriteRequest};

pub const DESCRIPTOR: u32 = 0x001D;
pub const IDENTIFY: u32 = 0x0003;
pub const ON_OFF: u32 = 0x0006;
pub const LEVEL_CONTROL: u32 = 0x0008;

/// Whether a client may write an attribute. Attributes that are read-only change only through their commands or
/// through the device itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    ReadWrite,
}

/// The shape of an attribute or field value. `Opaque` marks a value this model does not decode (such as a list); it
/// accepts no `AttributeValue` at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Bool,
    Uint { max: u32 },
    Opaque,
}

impl ValueKind {
    /// True when `value` fits this kind. An integer must be between zero and `max`, inclusive.
    pub fn accepts(&self, value: &AttributeValue) -> bool {
        match (self, value) {
            (ValueKind::Bool, AttributeValue::Bool(_)) => true,
            (ValueKind::Uint { max }, AttributeValue::Int(n)) => *n >= 0 && *n <= i64::from(*max),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributeSpec {
    pub id: u32,
    pub name: &'static str,
    pub kind: ValueKind,
    pub access: Access,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub id: u32,
    pub name: &'static str,
    pub kind: ValueKind,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub id: u32,
    pub name: &'static str,
    pub fields: &'static [FieldSpec],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterSpec {
    pub id: u32,
    pub name: &'static str,
    pub attributes: &'static [AttributeSpec],
    pub commands: &'static [CommandSpec],
}

impl ClusterSpec {
    pub fn attribute(&self, id: u32) -> Option<&'static AttributeSpec> {
        self.attributes.iter().find(|a| a.id == id)
    }

    pub fn command(&self, id: u32) -> Option<&'static CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }
}

/// Why a write or a command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterError {
    /// The path has a wildcard, so it names no single attribute or command.
    NotConcrete,
    UnknownCluster(u32),
    UnknownAttribute(u32),
    ReadOnlyAttribute(u32),
    /// The value does not fit the attribute's kind, or the attribute is opaque.
    WrongValue(u32),
    UnknownCommand(u32),
    UnknownField(u32),
    MissingField(u32),
    /// The field value does not fit the field's kind, or the field is opaque.
    WrongFieldValue(u32),
}

const IDENTIFY_ATTRIBUTES: &[AttributeSpec] = &[AttributeSpec {
    id: 0x0000,
    name: "IdentifyTime",
    kind: ValueKind::Uint { max: 0xFFFF },
    access: Access::ReadWrite,
}];

const IDENTIFY_COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: 0x00,
    name: "Identify",
    fields: &[FieldSpec { id: 0x0000, name: "IdentifyTime", kind: ValueKind::Uint { max: 0xFFFF }, required: true }],
}];

const DESCRIPTOR_ATTRIBUTES: &[AttributeSpec] = &[
    AttributeSpec { id: 0x0000, name: "DeviceTypeList", kind: ValueKind::Opaque, access: Access::Read },
    AttributeSpec { id: 0x0001, name: "ServerList", kind: ValueKind::Opaque, access: Access::Read },
    AttributeSpec { id: 0x0002, name: "ClientList", kind: ValueKind::Opaque, access: Access::Read },
    AttributeSpec { id: 0x0003, name: "PartsList", kind: ValueKind::Opaque, access: Access::Read },
];

const ON_OFF_ATTRIBUTES: &[AttributeSpec] =
    &[AttributeSpec { id: 0x0000, name: "OnOff", kind: ValueKind::Bool, access: Access::Read }];

const ON_OFF_COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: 0x00, name: "Off", fields: &[] },
    CommandSpec { id: 0x01, name: "On", fields: &[] },
    CommandSpec { id: 0x02, name: "Toggle", fields: &[] },
];

const LEVEL_ATTRIBUTES: &[AttributeSpec] = &[
    AttributeSpec { id: 0x0000, name: "CurrentLevel", kind: ValueKind::Uint { max: 254 }, access: Access::Read },
    AttributeSpec { id: 0x0001, name: "RemainingTime", kind: ValueKind::Uint { max: 0xFFFF }, access: Access::Read },
    AttributeSpec { id: 0x0002, name: "MinLevel", kind: ValueKind::Uint { max: 254 }, access: Access::Read },
    AttributeSpec { id: 0x0003, name: "MaxLevel", kind: ValueKind::Uint { max: 254 }, access: Access::Read },
];

const LEVEL_COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: 0x00,
    name: "MoveToLevel",
    fields: &[
        FieldSpec { id: 0x0000, name: "Level", kind: ValueKind::Uint { max: 254 }, required: true },
        FieldSpec { id: 0x0001, name: "TransitionTime", kind: ValueKind::Uint { max: 0xFFFF }, required: true },
        FieldSpec { id: 0x0002, name: "OptionsMask", kind: ValueKind::Uint { max: 0xFF }, required: true },
        FieldSpec { id: 0x0003, name: "OptionsOverride", kind: ValueKind::Uint { max: 0xFF }, required: true },
    ],
}];

static CLUSTERS: [ClusterSpec; 4] = [
    ClusterSpec { id: IDENTIFY, name: "Identify", attributes: IDENTIFY_ATTRIBUTES, commands: IDENTIFY_COMMANDS },
    ClusterSpec { id: ON_OFF, name: "OnOff", attributes: ON_OFF_ATTRIBUTES, commands: ON_OFF_COMMANDS },
    ClusterSpec { id: LEVEL_CONTROL, name: "LevelControl", attributes: LEVEL_ATTRIBUTES, commands: LEVEL_COMMANDS },
    ClusterSpec { id: DESCRIPTOR, name: "Descriptor", attributes: DESCRIPTOR_ATTRIBUTES, commands: &[] },
];

/// Every cluster in the model, in ID order.
pub fn clusters() -> &'static [ClusterSpec] {
    &CLUSTERS
}

/// The cluster with this ID, if the model has it.
pub fn cluster(id: u32) -> Option<&'static ClusterSpec> {
    CLUSTERS.iter().find(|c| c.id == id)
}

/// Checks a write against the cluster table: the cluster and attribute must exist, the attribute must be writable,
/// and the value must fit its kind.
pub fn check_write(req: &WriteRequest) -> Result<(), ClusterError> {
    let (_, cluster_id, attribute_id) = concrete(&req.path)?;
    let spec = cluster(cluster_id).ok_or(ClusterError::UnknownCluster(cluster_id))?;
    let attr = spec.attribute(attribute_id).ok_or(ClusterError::UnknownAttribute(attribute_id))?;
    if attr.access != Access::ReadWrite {
        return Err(ClusterError::ReadOnlyAttribute(attribute_id));
    }
    if !attr.kind.accepts(&req.value) {
        return Err(ClusterError::WrongValue(attribute_id));
    }
    Ok(())
}

/// Checks an invoke against the cluster table: the command must exist, every field must be known and fit its kind,
/// and every required field must be present.
pub fn check_command(req: &CommandRequest) -> Result<(), ClusterError> {
    let spec = cluster(req.path.cluster).ok_or(ClusterError::UnknownCluster(req.path.cluster))?;
    let command = spec.command(req.path.command).ok_or(ClusterError::UnknownCommand(req.path.command))?;
    for (field_id, value) in &req.fields {
        let field = command
            .fields
            .iter()
            .find(|f| f.id == *field_id)
            .ok_or(ClusterError::UnknownField(*field_id))?;
        if !field.kind.accepts(value) {
            return Err(ClusterError::WrongFieldValue(*field_id));
        }
    }
    for field in command.fields {
        if field.required && !req.fields.iter().any(|(id, _)| *id == field.id) {
            return Err(ClusterError::MissingField(field.id));
        }
    }
    Ok(())
}

fn concrete(path: &AttributePath) -> Result<(u32, u32, u32), ClusterError> {
    match (path.endpoint, path.cluster, path.attribute) {
        (Some(e), Some(c), Some(a)) => Ok((e, c, a)),
        _ => Err(ClusterError::NotConcrete),
    }
}

/// The On/Off cluster state. Commands change only the state; the attribute is read-only for clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct OnOffState {
    pub on: bool,
}

impl OnOffState {
    /// Applies an On/Off command. Returns `false`, and changes nothing, for a command ID the cluster does not have.
    pub fn apply(&mut self, command: u32) -> bool {
        match command {
            0x00 => self.on = false,
            0x01 => self.on = true,
            0x02 => self.on = !self.on,
            _ => return false,
        }
        true
    }
}

/// The Level Control state with its bounds. A move clamps the level to the minimum and maximum, as the cluster
/// requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelState {
    pub current: u8,
    pub min: u8,
    pub max: u8,
}

impl LevelState {
    /// Sets the level, clamped to `min..=max`. Returns the level that was set. A state whose minimum is above its
    /// maximum lands on the maximum, so the call never panics.
    pub fn move_to(&mut self, level: u8) -> u8 {
        self.current = level.max(self.min).min(self.max);
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matter_interaction::CommandPath;

    fn write(endpoint: Option<u32>, cluster: u32, attribute: u32, value: AttributeValue) -> WriteRequest {
        WriteRequest {
            path: AttributePath::new(endpoint, Some(cluster), Some(attribute)),
            value,
        }
    }

    #[test]
    fn the_model_lists_its_clusters_by_id() {
        assert_eq!(clusters().len(), 4);
        assert_eq!(cluster(ON_OFF).map(|c| c.name), Some("OnOff"));
        assert_eq!(cluster(LEVEL_CONTROL).map(|c| c.name), Some("LevelControl"));
        assert!(cluster(0xFFFF).is_none());
        let ids: Vec<u32> = clusters().iter().map(|c| c.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "clusters are in ID order");
    }

    #[test]
    fn writes_must_name_a_writable_attribute_with_a_fitting_value() {
        assert!(check_write(&write(Some(1), IDENTIFY, 0x0000, AttributeValue::Int(10))).is_ok());
        assert_eq!(
            check_write(&write(Some(1), IDENTIFY, 0x0000, AttributeValue::Int(0x1_0000))),
            Err(ClusterError::WrongValue(0x0000)),
            "IdentifyTime is 16 bits"
        );
        assert_eq!(
            check_write(&write(Some(1), IDENTIFY, 0x0000, AttributeValue::Int(-1))),
            Err(ClusterError::WrongValue(0x0000))
        );
        assert_eq!(
            check_write(&write(Some(1), IDENTIFY, 0x0000, AttributeValue::Bool(true))),
            Err(ClusterError::WrongValue(0x0000))
        );
        assert_eq!(
            check_write(&write(Some(1), ON_OFF, 0x0000, AttributeValue::Bool(true))),
            Err(ClusterError::ReadOnlyAttribute(0x0000)),
            "On/Off changes only through its commands"
        );
        assert_eq!(
            check_write(&write(Some(1), DESCRIPTOR, 0x0001, AttributeValue::Int(1))),
            Err(ClusterError::ReadOnlyAttribute(0x0001))
        );
        assert_eq!(
            check_write(&write(Some(1), 0xBEEF, 0, AttributeValue::Int(1))),
            Err(ClusterError::UnknownCluster(0xBEEF))
        );
        assert_eq!(
            check_write(&write(Some(1), IDENTIFY, 0x0042, AttributeValue::Int(1))),
            Err(ClusterError::UnknownAttribute(0x0042))
        );
    }

    #[test]
    fn a_write_with_a_wildcard_is_not_concrete() {
        let req = WriteRequest {
            path: AttributePath::new(None, Some(IDENTIFY), Some(0)),
            value: AttributeValue::Int(1),
        };
        assert_eq!(check_write(&req), Err(ClusterError::NotConcrete));
    }

    #[test]
    fn opaque_values_are_never_accepted() {
        assert!(!ValueKind::Opaque.accepts(&AttributeValue::Null));
        assert!(!ValueKind::Opaque.accepts(&AttributeValue::Int(0)));
        assert!(ValueKind::Bool.accepts(&AttributeValue::Bool(false)));
        assert!(!ValueKind::Bool.accepts(&AttributeValue::Int(1)));
    }

    #[test]
    fn move_to_level_takes_all_four_fields() {
        let path = CommandPath { endpoint: 1, cluster: LEVEL_CONTROL, command: 0x00 };
        let full = vec![
            (0x0000, AttributeValue::Int(128)),
            (0x0001, AttributeValue::Int(10)),
            (0x0002, AttributeValue::Int(0)),
            (0x0003, AttributeValue::Int(0)),
        ];
        let ok = CommandRequest::new(path, full.clone()).expect("distinct");
        assert!(check_command(&ok).is_ok());

        let missing = CommandRequest::new(path, full[..3].to_vec()).expect("distinct");
        assert_eq!(check_command(&missing), Err(ClusterError::MissingField(0x0003)));

        let too_big = CommandRequest::new(
            path,
            vec![
                (0x0000, AttributeValue::Int(255)),
                (0x0001, AttributeValue::Int(0)),
                (0x0002, AttributeValue::Int(0)),
                (0x0003, AttributeValue::Int(0)),
            ],
        )
        .expect("distinct");
        assert_eq!(check_command(&too_big), Err(ClusterError::WrongFieldValue(0x0000)));

        let stray = CommandRequest::new(path, {
            let mut more = full.clone();
            more.push((0x0009, AttributeValue::Int(0)));
            more
        })
        .expect("distinct");
        assert_eq!(check_command(&stray), Err(ClusterError::UnknownField(0x0009)));
    }

    #[test]
    fn on_off_commands_need_no_fields_and_unknown_ones_are_refused() {
        let on = CommandRequest::new(CommandPath { endpoint: 1, cluster: ON_OFF, command: 0x01 }, Vec::new())
            .expect("no fields");
        assert!(check_command(&on).is_ok());
        let bogus = CommandRequest::new(CommandPath { endpoint: 1, cluster: ON_OFF, command: 0x7F }, Vec::new())
            .expect("no fields");
        assert_eq!(check_command(&bogus), Err(ClusterError::UnknownCommand(0x7F)));
        let nowhere = CommandRequest::new(CommandPath { endpoint: 1, cluster: 0xBEEF, command: 0 }, Vec::new())
            .expect("no fields");
        assert_eq!(check_command(&nowhere), Err(ClusterError::UnknownCluster(0xBEEF)));
    }

    #[test]
    fn on_off_state_follows_on_off_and_toggle() {
        let mut state = OnOffState::default();
        assert!(!state.on);
        assert!(state.apply(0x01));
        assert!(state.on);
        assert!(state.apply(0x02));
        assert!(!state.on, "toggle flips the state");
        assert!(state.apply(0x00));
        assert!(!state.on);
        assert!(!state.apply(0x09), "unknown command changes nothing");
        assert!(!state.on);
    }

    #[test]
    fn level_moves_clamp_to_the_bounds() {
        let mut level = LevelState { current: 0, min: 10, max: 200 };
        assert_eq!(level.move_to(5), 10, "below the minimum clamps up");
        assert_eq!(level.move_to(250), 200, "above the maximum clamps down");
        assert_eq!(level.move_to(100), 100);
        assert_eq!(level.current, 100);
    }
}

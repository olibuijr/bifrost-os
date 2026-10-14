//! Controller initialization for Matter: the fabric a controller operates in, the node identifiers it hands out, and the
//! text form its credential record is kept in.
//!
//! A fabric is identified by a fabric identifier and a root public key (an uncompressed P-256 point, 65 bytes starting
//! with 0x04). Its compressed fabric identifier is derived from the key and the fabric identifier with HKDF-SHA256, as the
//! Matter specification describes. Pure: the caller supplies the key. Generating a key pair and signing operational
//! certificates need elliptic-curve code that core does not have yet, so they are not here.

use crate::hkdf::hkdf_sha256;
use crate::sha256::{hex, unhex};

/// Lowest and highest fabric identifiers a controller may use.
pub const FABRIC_ID_MIN: u64 = 1;
pub const FABRIC_ID_MAX: u64 = 0xFFFF_FFFF_FFFF_FFFE;
/// Lowest and highest operational node identifiers.
pub const NODE_ID_MIN: u64 = 1;
pub const NODE_ID_MAX: u64 = 0xFFFF_FFEF_FFFF_FFFF;
/// An uncompressed P-256 public key: the 0x04 marker, then X and Y.
pub const ROOT_KEY_LEN: usize = 65;
/// Length of the compressed fabric identifier.
pub const COMPRESSED_FABRIC_ID_LEN: usize = 8;

const COMPRESSED_FABRIC_INFO: &[u8] = b"CompressedFabric";

/// Why a fabric, a node identifier or a stored record was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FabricError {
    FabricIdOutOfRange(u64),
    NodeIdOutOfRange(u64),
    RootKeyLength(usize),
    RootKeyNotUncompressed,
    /// The HKDF output could not be produced.
    Derivation,
    /// Every operational node identifier in the range is in use.
    NodeIdsExhausted,
    /// A stored record names a different fabric from the one requested.
    FabricMismatch { stored: u64, requested: u64 },
    /// A stored record is missing a field or a field does not parse.
    Corrupt(String),
}

/// A fabric: its identifier, root public key and compressed identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fabric {
    fabric_id: u64,
    root_public_key: Vec<u8>,
    compressed_id: [u8; COMPRESSED_FABRIC_ID_LEN],
}

impl Fabric {
    /// Checks the identifier and the key, then derives the compressed identifier.
    pub fn new(fabric_id: u64, root_public_key: &[u8]) -> Result<Fabric, FabricError> {
        if !(FABRIC_ID_MIN..=FABRIC_ID_MAX).contains(&fabric_id) {
            return Err(FabricError::FabricIdOutOfRange(fabric_id));
        }
        if root_public_key.len() != ROOT_KEY_LEN {
            return Err(FabricError::RootKeyLength(root_public_key.len()));
        }
        if root_public_key[0] != 0x04 {
            return Err(FabricError::RootKeyNotUncompressed);
        }
        let compressed_id = compressed_fabric_id(fabric_id, &root_public_key[1..])?;
        Ok(Fabric { fabric_id, root_public_key: root_public_key.to_vec(), compressed_id })
    }

    pub fn fabric_id(&self) -> u64 {
        self.fabric_id
    }

    pub fn root_public_key(&self) -> &[u8] {
        &self.root_public_key
    }

    pub fn compressed_id(&self) -> [u8; COMPRESSED_FABRIC_ID_LEN] {
        self.compressed_id
    }
}

/// The compressed fabric identifier: HKDF-SHA256 over the root key's X and Y coordinates, salted with the fabric
/// identifier as eight big-endian bytes, expanded with the label "CompressedFabric".
pub fn compressed_fabric_id(fabric_id: u64, public_key_xy: &[u8]) -> Result<[u8; COMPRESSED_FABRIC_ID_LEN], FabricError> {
    let bytes = hkdf_sha256(public_key_xy, &fabric_id.to_be_bytes(), COMPRESSED_FABRIC_INFO, COMPRESSED_FABRIC_ID_LEN)
        .ok_or(FabricError::Derivation)?;
    <[u8; COMPRESSED_FABRIC_ID_LEN]>::try_from(bytes.as_slice()).map_err(|_| FabricError::Derivation)
}

/// A controller: its fabric and the next operational node identifier it will hand out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller {
    fabric: Fabric,
    next_node_id: u64,
}

impl Controller {
    /// Starts a controller on `fabric_id` with the key `root_public_key`. A stored record for the same fabric is
    /// reused, and its node allocation continues; a record for another fabric is refused, not replaced.
    pub fn initialize(stored: Option<Controller>, fabric_id: u64, root_public_key: &[u8]) -> Result<Controller, FabricError> {
        match stored {
            Some(existing) if existing.fabric.fabric_id != fabric_id => {
                Err(FabricError::FabricMismatch { stored: existing.fabric.fabric_id, requested: fabric_id })
            }
            Some(existing) => Ok(existing),
            None => Ok(Controller { fabric: Fabric::new(fabric_id, root_public_key)?, next_node_id: NODE_ID_MIN }),
        }
    }

    pub fn fabric(&self) -> &Fabric {
        &self.fabric
    }

    /// The node identifier the next commissioned device receives, without taking it.
    pub fn next_node_id(&self) -> u64 {
        self.next_node_id
    }

    /// Takes the next operational node identifier. Identifiers are never handed out twice.
    pub fn allocate_node_id(&mut self) -> Result<u64, FabricError> {
        if self.next_node_id > NODE_ID_MAX {
            return Err(FabricError::NodeIdsExhausted);
        }
        let id = self.next_node_id;
        self.next_node_id += 1;
        Ok(id)
    }

    /// The credential record as text: one `key=value` line per field, with the numbers in hexadecimal.
    pub fn encode(&self) -> String {
        format!(
            "fabric_id={:016x}\nroot_public_key={}\nnext_node_id={:016x}\n",
            self.fabric.fabric_id,
            hex(&self.fabric.root_public_key),
            self.next_node_id
        )
    }

    /// Reads a record written by [`Controller::encode`]. Every field is checked, and the derived identifier is rebuilt
    /// from the key, so a record cannot name a compressed identifier its key does not produce.
    pub fn decode(text: &str) -> Result<Controller, FabricError> {
        let fabric_id = u64::from_str_radix(&field(text, "fabric_id")?, 16)
            .map_err(|_| FabricError::Corrupt("fabric_id is not a number".into()))?;
        let key = unhex(&field(text, "root_public_key")?)
            .ok_or_else(|| FabricError::Corrupt("root_public_key is not hexadecimal".into()))?;
        let next_node_id = u64::from_str_radix(&field(text, "next_node_id")?, 16)
            .map_err(|_| FabricError::Corrupt("next_node_id is not a number".into()))?;
        if !(NODE_ID_MIN..=NODE_ID_MAX + 1).contains(&next_node_id) {
            return Err(FabricError::NodeIdOutOfRange(next_node_id));
        }
        Ok(Controller { fabric: Fabric::new(fabric_id, &key)?, next_node_id })
    }
}

fn field(text: &str, name: &str) -> Result<String, FabricError> {
    text.lines()
        .find_map(|line| line.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
        .map(|value| value.trim().to_string())
        .ok_or_else(|| FabricError::Corrupt(format!("missing {name}")))
}

//! `bifrost-audio-hal`: the client library (docs/audio-architecture.md, "Client IO", "Real-time threads"): the
//! object model's get, set and listen over the control socket, IO procedures on a real-time thread that the library
//! creates and the server promotes, and the device workgroup. The SDK re-exports it as `bifrost_app::audio_hardware`.
//!
//! Owner: task 06. The public API below is the lead's contract for the clients (tasks 07, 08, 09, 13 to 18).

use bifrost_audio_types::{AudioError, BufferList, ObjectId, PropertyAddress, PropertyValue, TimeStamp};

/// A connection to `bifrost-audiod` (one per process is enough; clone handles share it).
#[derive(Clone)]
pub struct System {}

/// The default-device roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Output,
    Input,
    SoundEffects,
}

/// A listener registration; dropping it unlistens.
pub struct Listener {}

impl System {
    /// Connects to the control socket and says hello as `client` (the app id).
    pub fn connect(client: &str) -> Result<System, AudioError> {
        let _ = client;
        todo!("audio-06")
    }

    pub fn default_device(&self, role: Role) -> Result<Device, AudioError> {
        let _ = role;
        todo!("audio-06")
    }

    pub fn devices(&self) -> Result<Vec<Device>, AudioError> {
        todo!("audio-06")
    }

    pub fn get(&self, object: ObjectId, address: PropertyAddress) -> Result<PropertyValue, AudioError> {
        let _ = (object, address);
        todo!("audio-06")
    }

    pub fn set(&self, object: ObjectId, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        let _ = (object, address, value);
        todo!("audio-06")
    }

    /// Calls `f` (on the library's notification thread) when the property changes; re-read the value in `f`.
    pub fn listen(&self, object: ObjectId, address: PropertyAddress, f: Box<dyn Fn(ObjectId, PropertyAddress) + Send + Sync>) -> Result<Listener, AudioError> {
        let _ = (object, address, f);
        todo!("audio-06")
    }
}

/// A device handle.
#[derive(Clone)]
pub struct Device {
    pub id: ObjectId,
    pub system: System,
}

/// What an IO procedure gets each cycle (the reference IOProc, made safe). It runs on a real-time thread: no
/// allocation, no locks, no syscalls other than the futex (enforced in debug builds).
pub struct IoContext {
    pub cycle: u64,
    /// Frames this call covers (the client's buffer size).
    pub frames: u32,
}

/// The IO procedure: `now`, the input of this cycle with its timestamp, and the output to fill with its timestamp.
pub type IoProc = Box<dyn FnMut(&IoContext, &TimeStamp, &BufferList, &TimeStamp, &mut BufferList, &TimeStamp) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IoProcId(pub u32);

/// How a client wants to run on a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoProcConfig {
    pub input_channels: u32,
    pub output_channels: u32,
    /// 0 means the device's current buffer size.
    pub buffer_frames: u32,
}

impl Device {
    pub fn add_io_proc(&self, cfg: IoProcConfig, f: IoProc) -> Result<IoProcId, AudioError> {
        let _ = (cfg, f);
        todo!("audio-06")
    }

    pub fn start(&self, id: IoProcId) -> Result<(), AudioError> {
        let _ = id;
        todo!("audio-06")
    }

    pub fn stop(&self, id: IoProcId) -> Result<(), AudioError> {
        let _ = id;
        todo!("audio-06")
    }

    pub fn remove_io_proc(&self, id: IoProcId) -> Result<(), AudioError> {
        let _ = id;
        todo!("audio-06")
    }

    pub fn get(&self, address: PropertyAddress) -> Result<PropertyValue, AudioError> {
        self.system.get(self.id, address)
    }

    pub fn set(&self, address: PropertyAddress, value: PropertyValue) -> Result<(), AudioError> {
        self.system.set(self.id, address, value)
    }
}

/// The device workgroup: real-time threads that share the device's deadline (SCHED_FIFO through the server,
/// `sched_util_min` while the device runs, CPU affinity hints). Owner: task 06.
pub mod workgroup;

/// Hog mode requests (owner: task 10).
pub mod hog;

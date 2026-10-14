//! Devices: the account's devices, each with its last known position and battery level, a lost mode and a queue of
//! commands. The model sends nothing itself: the cloud sync takes the queued commands with `take_commands`, and a
//! position reaches a device only through `report`, which keeps the newest fix.

/// A position fix in degrees, with its accuracy in metres and the time it was taken (Unix seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy_m: f64,
    pub at_unix: i64,
}

/// Lost mode: the device is locked and shows a message and a contact number on its lock screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostMode {
    pub message: String,
    pub phone: String,
}

/// A command the cloud delivers to a device on its next sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    PlaySound,
    Lock,
    Erase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    /// Erasing a device needs the person to confirm it.
    NeedsConfirmation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub position: Option<Fix>,
    pub battery: Option<u8>,
    pub lost: Option<LostMode>,
    pending: Vec<Command>,
}

impl Device {
    pub fn new(id: &str, name: &str) -> Device {
        Device {
            id: id.trim().to_string(),
            name: name.trim().to_string(),
            position: None,
            battery: None,
            lost: None,
            pending: Vec::new(),
        }
    }

    /// Stores a fix when it is newer than the one held. Returns whether it was stored.
    pub fn report(&mut self, fix: Fix) -> bool {
        match self.position {
            Some(held) if held.at_unix >= fix.at_unix => false,
            _ => {
                self.position = Some(fix);
                true
            }
        }
    }

    /// Records the battery level, clamped to 100 percent.
    pub fn set_battery(&mut self, percent: u8) {
        self.battery = Some(percent.min(100));
    }

    /// Turns lost mode on. The device locks, and its lock screen shows the message and the number.
    pub fn mark_lost(&mut self, message: &str, phone: &str) {
        self.lost = Some(LostMode { message: message.trim().to_string(), phone: phone.trim().to_string() });
        self.queue(Command::Lock);
    }

    pub fn clear_lost(&mut self) {
        self.lost = None;
    }

    /// Queues a command for the next sync. Play Sound and Lock queue at once; Erase queues only when `confirmed`.
    /// A command that is already waiting is not queued twice.
    pub fn request(&mut self, command: Command, confirmed: bool) -> Result<(), CommandError> {
        if command == Command::Erase && !confirmed {
            return Err(CommandError::NeedsConfirmation);
        }
        self.queue(command);
        Ok(())
    }

    fn queue(&mut self, command: Command) {
        if !self.pending.contains(&command) {
            self.pending.push(command);
        }
    }

    /// The commands waiting for the next sync, in the order they were requested.
    pub fn pending(&self) -> &[Command] {
        &self.pending
    }

    /// Hands the queued commands to the sync and empties the queue.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.pending)
    }

    /// The battery as the device list shows it, or nothing when the level is unknown.
    pub fn battery_label(&self) -> Option<String> {
        self.battery.map(|p| format!("Battery {p}%"))
    }
}

/// The devices on the account, in the order the cloud lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Devices {
    list: Vec<Device>,
}

impl Devices {
    pub fn new(list: Vec<Device>) -> Devices {
        Devices { list }
    }

    pub fn list(&self) -> &[Device] {
        &self.list
    }

    pub fn get(&self, id: &str) -> Option<&Device> {
        self.list.iter().find(|d| d.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Device> {
        self.list.iter_mut().find(|d| d.id == id)
    }
}

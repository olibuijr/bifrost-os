//! Live broadcast: a picker that chooses one broadcast service and checks its ingest address, and a controller that
//! starts, pauses, resumes and finishes a broadcast to that service. The controller calls a handler on each change, in
//! the order the reference broadcast controller reports them. Pure state: no frame is captured or sent. The shell has
//! no broadcast extension point yet, so the handler receives lifecycle events only.

/// Why a broadcast request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The service name is empty after trimming, or longer than 64 characters.
    BadName,
    /// The ingest address is not an https, rtmp or rtmps address with a host and no whitespace.
    BadUrl,
    /// The picker has no service selected.
    NoSelection,
    /// A broadcast is already running or paused.
    AlreadyBroadcasting,
    /// The action needs a running or paused broadcast.
    NotBroadcasting,
    /// Resume needs a paused broadcast.
    NotPaused,
}

const MAX_NAME_LEN: usize = 64;

/// A broadcast service and where its stream goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BroadcastConfiguration {
    service_name: String,
    url: String,
}

impl BroadcastConfiguration {
    /// Checks the name and the ingest address; both are stored trimmed.
    pub fn new(service_name: &str, url: &str) -> Result<Self, Error> {
        let name = service_name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
            return Err(Error::BadName);
        }
        let url = url.trim();
        if !valid_ingest_url(url) {
            return Err(Error::BadUrl);
        }
        Ok(Self { service_name: name.to_string(), url: url.to_string() })
    }

    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

fn valid_ingest_url(url: &str) -> bool {
    let rest = ["https://", "rtmp://", "rtmps://"].into_iter().find_map(|scheme| url.strip_prefix(scheme));
    match rest {
        Some(rest) => {
            let host = rest.split(|c: char| matches!(c, '/' | ':' | '?')).next().unwrap_or("");
            !host.is_empty() && !url.contains(char::is_whitespace)
        }
        None => false,
    }
}

/// One service the picker offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BroadcastService {
    pub name: String,
    pub url: String,
}

/// The list of services and the one the person chose.
#[derive(Clone, Debug, Default)]
pub struct BroadcastPicker {
    services: Vec<BroadcastService>,
    choice: Option<usize>,
}

impl BroadcastPicker {
    /// A picker with nothing selected.
    pub fn new(services: Vec<BroadcastService>) -> Self {
        Self { services, choice: None }
    }

    pub fn services(&self) -> &[BroadcastService] {
        &self.services
    }

    /// Selects the service at `index`. An index past the end is refused and leaves the selection unchanged.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.services.len() {
            self.choice = Some(index);
            true
        } else {
            false
        }
    }

    pub fn selected(&self) -> Option<&BroadcastService> {
        self.choice.and_then(|index| self.services.get(index))
    }

    /// The configuration of the selected service, checked the same way as [`BroadcastConfiguration::new`].
    pub fn confirm(&self) -> Result<BroadcastConfiguration, Error> {
        let service = self.selected().ok_or(Error::NoSelection)?;
        BroadcastConfiguration::new(&service.name, &service.url)
    }
}

/// The lifecycle callbacks of one broadcast, called by [`BroadcastController`] after each state change.
pub trait BroadcastHandler {
    fn started(&mut self, configuration: &BroadcastConfiguration);
    fn paused(&mut self);
    fn resumed(&mut self);
    fn finished(&mut self);
}

/// Where a broadcast is in its life. A finished broadcast may start again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Broadcasting,
    Paused,
    Finished,
}

/// Drives one broadcast at a time and reports each change to its handler.
pub struct BroadcastController<H: BroadcastHandler> {
    handler: H,
    state: State,
    configuration: Option<BroadcastConfiguration>,
}

impl<H: BroadcastHandler> BroadcastController<H> {
    pub fn new(handler: H) -> Self {
        Self { handler, state: State::Idle, configuration: None }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// The configuration of the latest broadcast, kept after it finishes.
    pub fn configuration(&self) -> Option<&BroadcastConfiguration> {
        self.configuration.as_ref()
    }

    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Starts a broadcast. Refused while one is running or paused.
    pub fn start(&mut self, configuration: BroadcastConfiguration) -> Result<(), Error> {
        if matches!(self.state, State::Broadcasting | State::Paused) {
            return Err(Error::AlreadyBroadcasting);
        }
        self.handler.started(&configuration);
        self.configuration = Some(configuration);
        self.state = State::Broadcasting;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), Error> {
        if self.state != State::Broadcasting {
            return Err(Error::NotBroadcasting);
        }
        self.state = State::Paused;
        self.handler.paused();
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), Error> {
        if self.state != State::Paused {
            return Err(Error::NotPaused);
        }
        self.state = State::Broadcasting;
        self.handler.resumed();
        Ok(())
    }

    /// Ends the running or paused broadcast.
    pub fn finish(&mut self) -> Result<(), Error> {
        if !matches!(self.state, State::Broadcasting | State::Paused) {
            return Err(Error::NotBroadcasting);
        }
        self.state = State::Finished;
        self.handler.finished();
        Ok(())
    }
}

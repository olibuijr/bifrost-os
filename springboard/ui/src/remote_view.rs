//! A view that another process draws and the host shows in its own window. The host asks for a connection, keeps the
//! view inside the bounds its layout allows, and shows a placeholder until the view is up or after it is lost for good.
//! This is the host's side of the state machine only: no sockets, no processes, no clock. The shell connects the
//! transport and reports what happens through [`Session::connected`] and [`Session::lost`].

/// Where a remote view is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not connected, and nothing has asked for a connection.
    Idle,
    /// A connection was asked for and is not up yet.
    Connecting,
    /// The remote view is on screen.
    Active,
    /// The connection was lost and the host may try again. `attempt` counts losses since the last good connection.
    Reconnecting { attempt: u32 },
    /// Lost more times in a row than the host allows. Stop, then connect again, to start over.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A connection is already up or being made.
    Busy,
}

/// The size range the host lays the view out in, in points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub min_w: u32,
    pub min_h: u32,
    pub max_w: u32,
    pub max_h: u32,
}

impl Bounds {
    /// `None` when a minimum is larger than its maximum.
    pub fn new(min_w: u32, min_h: u32, max_w: u32, max_h: u32) -> Option<Bounds> {
        if min_w > max_w || min_h > max_h {
            return None;
        }
        Some(Bounds { min_w, min_h, max_w, max_h })
    }

    /// The nearest size inside the bounds.
    pub fn clamp(&self, w: u32, h: u32) -> (u32, u32) {
        (w.clamp(self.min_w, self.max_w), h.clamp(self.min_h, self.max_h))
    }
}

/// One remote view as the host tracks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    bounds: Bounds,
    max_losses: u32,
    /// Losses since the last successful connection.
    losses: u32,
    phase: Phase,
    size: (u32, u32),
}

impl Session {
    /// A new, idle session showing the smallest size. `max_losses` is how many losses in a row the host tolerates
    /// before it gives up.
    pub fn new(bounds: Bounds, max_losses: u32) -> Self {
        let size = (bounds.min_w, bounds.min_h);
        Session { bounds, max_losses, losses: 0, phase: Phase::Idle, size }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The size the view is shown at.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The host shows its placeholder in every phase except active.
    pub fn shows_placeholder(&self) -> bool {
        self.phase != Phase::Active
    }

    /// Asks for a connection. Allowed from idle and while reconnecting.
    pub fn connect(&mut self) -> Result<(), Error> {
        match self.phase {
            Phase::Idle | Phase::Reconnecting { .. } => {
                self.phase = Phase::Connecting;
                Ok(())
            }
            _ => Err(Error::Busy),
        }
    }

    /// The transport is up. Returns false, and changes nothing, when no connection was being made.
    pub fn connected(&mut self) -> bool {
        if self.phase != Phase::Connecting {
            return false;
        }
        self.phase = Phase::Active;
        self.losses = 0;
        true
    }

    /// The connection dropped or never came up. Returns the new phase. A loss while idle or failed changes nothing.
    pub fn lost(&mut self) -> Phase {
        if !matches!(self.phase, Phase::Connecting | Phase::Active) {
            return self.phase;
        }
        self.losses += 1;
        self.phase = if self.losses > self.max_losses {
            Phase::Failed
        } else {
            Phase::Reconnecting { attempt: self.losses }
        };
        self.phase
    }

    /// Drops the connection on purpose and clears the loss count.
    pub fn stop(&mut self) {
        self.phase = Phase::Idle;
        self.losses = 0;
    }

    /// The host lays the view out at `w` by `h`. The size is clamped to the bounds and returned.
    pub fn fit(&mut self, w: u32, h: u32) -> (u32, u32) {
        self.size = self.bounds.clamp(w, h);
        self.size
    }

    /// The remote view asks for `w` by `h`. Only an active view may resize itself. Returns the new size when it
    /// changed, `None` when it did not or the view is not active.
    pub fn report(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.phase != Phase::Active {
            return None;
        }
        let next = self.bounds.clamp(w, h);
        if next == self.size {
            return None;
        }
        self.size = next;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds {
        Bounds::new(100, 50, 400, 300).unwrap()
    }

    #[test]
    fn a_new_session_is_idle_shows_the_placeholder_at_the_minimum_size() {
        let s = Session::new(bounds(), 2);
        assert_eq!(s.phase(), Phase::Idle);
        assert!(s.shows_placeholder());
        assert_eq!(s.size(), (100, 50));
    }

    #[test]
    fn connecting_then_connected_shows_the_view_and_a_second_connect_is_busy() {
        let mut s = Session::new(bounds(), 2);
        assert_eq!(s.connect(), Ok(()));
        assert_eq!(s.phase(), Phase::Connecting);
        assert!(s.connected());
        assert_eq!(s.phase(), Phase::Active);
        assert!(!s.shows_placeholder());
        assert!(!s.connected());
        assert_eq!(s.connect(), Err(Error::Busy));
    }

    #[test]
    fn a_good_connection_resets_the_loss_count() {
        let mut s = Session::new(bounds(), 2);
        s.connect().unwrap();
        s.connected();
        assert_eq!(s.lost(), Phase::Reconnecting { attempt: 1 });
        s.connect().unwrap();
        s.connected();
        assert_eq!(s.lost(), Phase::Reconnecting { attempt: 1 });
    }

    #[test]
    fn losses_in_a_row_past_the_limit_fail_until_the_host_stops() {
        let mut s = Session::new(bounds(), 2);
        s.connect().unwrap();
        assert_eq!(s.lost(), Phase::Reconnecting { attempt: 1 });
        s.connect().unwrap();
        assert_eq!(s.lost(), Phase::Reconnecting { attempt: 2 });
        s.connect().unwrap();
        assert_eq!(s.lost(), Phase::Failed);
        assert_eq!(s.lost(), Phase::Failed);
        assert_eq!(s.connect(), Err(Error::Busy));
        s.stop();
        assert_eq!(s.phase(), Phase::Idle);
        assert_eq!(s.connect(), Ok(()));
    }

    #[test]
    fn a_loss_while_idle_changes_nothing() {
        let mut s = Session::new(bounds(), 2);
        assert_eq!(s.lost(), Phase::Idle);
        assert!(!s.connected());
    }

    #[test]
    fn a_fit_is_clamped_to_the_bounds_and_a_report_only_counts_when_active_and_changed() {
        let mut s = Session::new(bounds(), 2);
        assert_eq!(s.fit(1000, 120), (400, 120));
        assert_eq!(s.size(), (400, 120));
        // Not active yet: the view may not resize itself.
        assert_eq!(s.report(200, 200), None);
        s.connect().unwrap();
        s.connected();
        assert_eq!(s.report(500, 20), Some((400, 50)));
        assert_eq!(s.report(400, 50), None);
        assert_eq!(s.report(10, 10), Some((100, 50)));
    }

    #[test]
    fn bounds_with_a_minimum_above_the_maximum_are_refused() {
        assert_eq!(Bounds::new(200, 50, 100, 300), None);
        assert_eq!(Bounds::new(200, 400, 300, 300), None);
        assert_eq!(Bounds::new(100, 50, 100, 50).map(|b| b.clamp(7, 9)), Some((100, 50)));
    }
}

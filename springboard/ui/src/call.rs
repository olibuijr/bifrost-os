//! Call state for a calling service: calls with a handle and a state, the requests a provider applies to them as one
//! transaction, and the events an observer reads back (`NativeCallController`, `NativeCallTransaction`,
//! `NativeCallObserver`). Ringing, audio and the call screen belong to the shell; this module only decides what a
//! request does to a call.

/// How the other party is reached (`NativeCallHandle`): a phone number, an email address or an app-specific id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handle {
    Phone(String),
    Email(String),
    Generic(String),
}

impl Handle {
    /// The value as the caller is shown on a prompt.
    pub fn display(&self) -> &str {
        match self {
            Handle::Phone(value) | Handle::Email(value) | Handle::Generic(value) => value,
        }
    }

    /// Checks the value for its kind. A phone number is an optional leading `+` and 3 to 20 digits. An email needs
    /// text on both sides of one `@`. A generic handle only has to be non-empty.
    pub fn validate(&self) -> Result<(), CallError> {
        let ok = match self {
            Handle::Phone(value) => {
                let digits = value.strip_prefix('+').unwrap_or(value.as_str());
                (3..=20).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
            }
            Handle::Email(value) => match value.split_once('@') {
                Some((local, domain)) => !local.is_empty() && !domain.is_empty() && !domain.contains('@'),
                None => false,
            },
            Handle::Generic(value) => !value.is_empty(),
        };
        if ok {
            Ok(())
        } else {
            Err(CallError::InvalidHandle)
        }
    }
}

/// Where a call is. A call that ends leaves the active list, so `Ended` is never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallState {
    /// An outgoing call that the other party has not answered yet.
    Dialing,
    /// An incoming call that waits to be answered.
    Ringing,
    Connected,
    Held,
    /// The call has ended; only seen in the state of a request that ends a call.
    Ended,
}

/// One active call (`NativeCall`). `muted` is the local microphone state and survives hold. `video` and
/// `contact_identifier` come from the start action of an outgoing call; other calls keep `false` and `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub id: u64,
    pub handle: Handle,
    pub outgoing: bool,
    pub state: CallState,
    pub muted: bool,
    pub video: bool,
    pub contact_identifier: Option<String>,
}

/// A request to start an outgoing call (`NativeStartCallAction`): the recipient, whether the call is a video call,
/// and the identifier of the contact the recipient was picked from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartCallAction {
    pub handle: Handle,
    pub is_video: bool,
    pub contact_identifier: Option<String>,
}

/// One request to change a call (`NativeCallAction`). A transaction applies a list of them in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallAction {
    /// Answers a ringing call.
    Answer,
    /// Ends the call from any state.
    End,
    /// Places a connected call on hold, or resumes a held one.
    SetHeld(bool),
    /// Mutes or unmutes the local microphone. Allowed in every state, so a call can be muted before it is answered.
    SetMuted(bool),
    /// Sends DTMF tones on a connected call. Digits are `0`-`9`, `*`, `#` and uppercase `A`-`D`. The state does not
    /// change; tone output belongs to the audio service.
    PlayDtmf(String),
}

/// Why a request was refused. A refused request changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    UnknownCall(u64),
    InvalidHandle,
    /// The action does not apply in the call's current state.
    InvalidState,
    InvalidDigits,
}

/// What changed, for an observer (`NativeCallObserver` delegate calls). `Changed` is sent only when a call differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallEvent {
    Added(u64),
    Changed(u64),
    Ended(u64),
}

/// How an incoming call is shown. A banner sits over the desktop; a full-screen prompt is used while the device is locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptStyle {
    Banner,
    FullScreen,
}

/// The prompt for a ringing incoming call: who is calling, how it is shown, and the two buttons. Answering uses
/// `CallAction::Answer`; declining uses `CallAction::End`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingPrompt {
    pub call_id: u64,
    pub style: PromptStyle,
    pub caller: String,
    pub answer: CallAction,
    pub decline: CallAction,
}

/// Builds the prompt for a call. Only a ringing incoming call gets one; outgoing calls and calls already answered
/// or on hold return `None`.
pub fn incoming_prompt(call: &Call, device_locked: bool) -> Option<IncomingPrompt> {
    if call.outgoing || call.state != CallState::Ringing {
        return None;
    }
    Some(IncomingPrompt {
        call_id: call.id,
        style: if device_locked { PromptStyle::FullScreen } else { PromptStyle::Banner },
        caller: call.handle.display().to_string(),
        answer: CallAction::Answer,
        decline: CallAction::End,
    })
}

fn valid_dtmf(digits: &str) -> bool {
    !digits.is_empty() && digits.chars().all(|c| matches!(c, '0'..='9' | '*' | '#' | 'A'..='D'))
}

/// The active calls and the provider that changes them (`NativeCallController` with its `NativeCallObserver` list).
/// Events collect until `take_events` reads them.
#[derive(Debug, Default)]
pub struct CallCenter {
    calls: Vec<Call>,
    next_id: u64,
    events: Vec<CallEvent>,
}

impl CallCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an outgoing call in the dialing state (`NativeStartCallAction`).
    pub fn start_outgoing(&mut self, handle: Handle) -> Result<u64, CallError> {
        handle.validate()?;
        Ok(self.add(handle, true, CallState::Dialing))
    }

    /// Starts an outgoing call from a start action. The call keeps the video flag and the contact identifier. A refused
    /// handle starts nothing.
    pub fn start_action(&mut self, action: StartCallAction) -> Result<u64, CallError> {
        let StartCallAction { handle, is_video, contact_identifier } = action;
        handle.validate()?;
        let id = self.add(handle, true, CallState::Dialing);
        if let Some(call) = self.calls.iter_mut().find(|c| c.id == id) {
            call.video = is_video;
            call.contact_identifier = contact_identifier;
        }
        Ok(id)
    }

    /// Reports an incoming call that rings until it is answered or ended (`reportNewIncomingCall`).
    pub fn report_incoming(&mut self, handle: Handle) -> Result<u64, CallError> {
        handle.validate()?;
        Ok(self.add(handle, false, CallState::Ringing))
    }

    /// The other party answered an outgoing call (`reportOutgoingCall(connectedAt:)`). Only a dialing call can connect.
    pub fn report_connected(&mut self, id: u64) -> Result<(), CallError> {
        let index = self.index_of(id)?;
        if self.calls[index].state != CallState::Dialing {
            return Err(CallError::InvalidState);
        }
        self.calls[index].state = CallState::Connected;
        self.events.push(CallEvent::Changed(id));
        Ok(())
    }

    /// Answers or declines a ringing call from its prompt. Declining ends the call. A refused answer changes nothing.
    pub fn respond_incoming(&mut self, id: u64, answer: bool) -> Result<(), CallError> {
        let action = if answer { CallAction::Answer } else { CallAction::End };
        self.perform(id, action)
    }

    /// Applies one action to one call. Same as a transaction with one action.
    pub fn perform(&mut self, id: u64, action: CallAction) -> Result<(), CallError> {
        self.perform_transaction(vec![(id, action)])
    }

    /// Applies the actions in order as one unit (`NativeCallTransaction`). If any action is refused, no call changes
    /// and no event is sent.
    pub fn perform_transaction(&mut self, actions: Vec<(u64, CallAction)>) -> Result<(), CallError> {
        let mut staged = self.calls.clone();
        let mut events = Vec::new();
        for (id, action) in actions {
            let index = staged.iter().position(|c| c.id == id).ok_or(CallError::UnknownCall(id))?;
            let next = Self::apply(&staged[index], action)?;
            if next.state == CallState::Ended {
                staged.remove(index);
                events.push(CallEvent::Ended(id));
            } else if next != staged[index] {
                staged[index] = next;
                events.push(CallEvent::Changed(id));
            }
        }
        self.calls = staged;
        self.events.extend(events);
        Ok(())
    }

    /// The active calls, in the order they were started or reported.
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    pub fn call(&self, id: u64) -> Option<&Call> {
        self.calls.iter().find(|c| c.id == id)
    }

    /// Returns the events since the last read, oldest first.
    pub fn take_events(&mut self) -> Vec<CallEvent> {
        std::mem::take(&mut self.events)
    }

    fn add(&mut self, handle: Handle, outgoing: bool, state: CallState) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.calls.push(Call { id, handle, outgoing, state, muted: false, video: false, contact_identifier: None });
        self.events.push(CallEvent::Added(id));
        id
    }

    fn index_of(&self, id: u64) -> Result<usize, CallError> {
        self.calls.iter().position(|c| c.id == id).ok_or(CallError::UnknownCall(id))
    }

    /// The call that results from `action`, or why the action does not apply in the call's state.
    fn apply(call: &Call, action: CallAction) -> Result<Call, CallError> {
        let mut next = call.clone();
        match (call.state, action) {
            (_, CallAction::End) => next.state = CallState::Ended,
            (CallState::Ringing, CallAction::Answer) => next.state = CallState::Connected,
            (CallState::Connected, CallAction::SetHeld(true)) => next.state = CallState::Held,
            (CallState::Held, CallAction::SetHeld(false)) => next.state = CallState::Connected,
            (_, CallAction::SetMuted(muted)) => next.muted = muted,
            (CallState::Connected, CallAction::PlayDtmf(digits)) => {
                if !valid_dtmf(&digits) {
                    return Err(CallError::InvalidDigits);
                }
            }
            _ => return Err(CallError::InvalidState),
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(number: &str) -> Handle {
        Handle::Phone(number.to_string())
    }

    #[test]
    fn handles_are_checked_for_their_kind() {
        assert_eq!(phone("+3545551234").validate(), Ok(()));
        assert_eq!(phone("12").validate(), Err(CallError::InvalidHandle));
        assert_eq!(phone("12a4567").validate(), Err(CallError::InvalidHandle));
        assert_eq!(phone("+").validate(), Err(CallError::InvalidHandle));
        assert_eq!(Handle::Email("ann@example.is".into()).validate(), Ok(()));
        assert_eq!(Handle::Email("ann@".into()).validate(), Err(CallError::InvalidHandle));
        assert_eq!(Handle::Email("a@b@c".into()).validate(), Err(CallError::InvalidHandle));
        assert_eq!(Handle::Generic(String::new()).validate(), Err(CallError::InvalidHandle));
    }

    #[test]
    fn outgoing_call_connects_then_ends_once() {
        let mut center = CallCenter::new();
        let id = center.start_outgoing(phone("5551234")).unwrap();
        assert_eq!(center.call(id).unwrap().state, CallState::Dialing);
        assert!(center.call(id).unwrap().outgoing);

        center.report_connected(id).unwrap();
        assert_eq!(center.report_connected(id), Err(CallError::InvalidState));
        center.perform(id, CallAction::End).unwrap();

        assert!(center.calls().is_empty());
        assert_eq!(center.perform(id, CallAction::End), Err(CallError::UnknownCall(id)));
        assert_eq!(
            center.take_events(),
            vec![CallEvent::Added(id), CallEvent::Changed(id), CallEvent::Ended(id)]
        );
        assert!(center.take_events().is_empty());
    }

    #[test]
    fn start_action_keeps_video_and_contact_and_refuses_a_bad_handle() {
        let mut center = CallCenter::new();
        let id = center
            .start_action(StartCallAction {
                handle: phone("5551234"),
                is_video: true,
                contact_identifier: Some("contact-7".into()),
            })
            .unwrap();
        let call = center.call(id).unwrap();
        assert_eq!(call.state, CallState::Dialing);
        assert!(call.outgoing);
        assert!(call.video);
        assert_eq!(call.contact_identifier.as_deref(), Some("contact-7"));

        let refused = center.start_action(StartCallAction {
            handle: phone("12"),
            is_video: false,
            contact_identifier: None,
        });
        assert_eq!(refused, Err(CallError::InvalidHandle));
        assert_eq!(center.calls().len(), 1);

        // A plain outgoing call has no video and no contact.
        let plain = center.start_outgoing(phone("5559999")).unwrap();
        assert!(!center.call(plain).unwrap().video);
        assert_eq!(center.call(plain).unwrap().contact_identifier, None);
    }

    #[test]
    fn incoming_call_answers_and_hold_round_trips() {
        let mut center = CallCenter::new();
        let id = center.report_incoming(phone("5551234")).unwrap();
        assert_eq!(center.perform(id, CallAction::SetHeld(true)), Err(CallError::InvalidState));

        center.perform(id, CallAction::Answer).unwrap();
        center.perform(id, CallAction::SetHeld(true)).unwrap();
        assert_eq!(center.call(id).unwrap().state, CallState::Held);
        assert_eq!(center.perform(id, CallAction::Answer), Err(CallError::InvalidState));
        center.perform(id, CallAction::SetHeld(false)).unwrap();
        assert_eq!(center.call(id).unwrap().state, CallState::Connected);
    }

    #[test]
    fn muting_works_before_the_call_is_answered_and_survives_hold() {
        let mut center = CallCenter::new();
        let id = center.report_incoming(phone("5551234")).unwrap();
        center.take_events();
        center.perform(id, CallAction::SetMuted(true)).unwrap();
        assert_eq!(center.call(id).unwrap().state, CallState::Ringing);
        assert!(center.call(id).unwrap().muted);
        assert_eq!(center.take_events(), vec![CallEvent::Changed(id)]);

        // Repeating the same mute changes nothing, so no event is sent.
        center.perform(id, CallAction::SetMuted(true)).unwrap();
        assert!(center.take_events().is_empty());

        center.perform(id, CallAction::Answer).unwrap();
        center.perform(id, CallAction::SetHeld(true)).unwrap();
        assert!(center.call(id).unwrap().muted);
    }

    #[test]
    fn transaction_is_all_or_nothing() {
        let mut center = CallCenter::new();
        let a = center.report_incoming(phone("5551111")).unwrap();
        center.perform(a, CallAction::Answer).unwrap();
        let b = center.report_incoming(phone("5552222")).unwrap();
        center.take_events();

        // B is ringing, so the tone request fails and A must not be muted.
        let refused = center.perform_transaction(vec![
            (a, CallAction::SetMuted(true)),
            (b, CallAction::PlayDtmf("1".into())),
        ]);
        assert_eq!(refused, Err(CallError::InvalidState));
        assert!(!center.call(a).unwrap().muted);
        assert!(center.take_events().is_empty());

        center
            .perform_transaction(vec![(a, CallAction::SetMuted(true)), (b, CallAction::Answer)])
            .unwrap();
        assert!(center.call(a).unwrap().muted);
        assert_eq!(center.call(b).unwrap().state, CallState::Connected);
        assert_eq!(center.take_events(), vec![CallEvent::Changed(a), CallEvent::Changed(b)]);
    }

    #[test]
    fn dtmf_accepts_keypad_symbols_only_on_connected_calls() {
        let mut center = CallCenter::new();
        let ringing = center.report_incoming(phone("5551111")).unwrap();
        let id = center.report_incoming(phone("5552222")).unwrap();
        center.perform(id, CallAction::Answer).unwrap();

        assert_eq!(center.perform(id, CallAction::PlayDtmf("12*#ABCD".into())), Ok(()));
        assert_eq!(center.perform(id, CallAction::PlayDtmf(String::new())), Err(CallError::InvalidDigits));
        assert_eq!(center.perform(id, CallAction::PlayDtmf("1x".into())), Err(CallError::InvalidDigits));
        assert_eq!(center.perform(id, CallAction::PlayDtmf("e".into())), Err(CallError::InvalidDigits));
        assert_eq!(center.perform(ringing, CallAction::PlayDtmf("1".into())), Err(CallError::InvalidState));
    }

    #[test]
    fn ending_a_call_removes_only_that_call() {
        let mut center = CallCenter::new();
        let first = center.report_incoming(phone("5551111")).unwrap();
        let second = center.report_incoming(phone("5552222")).unwrap();
        center.take_events();

        center.perform(first, CallAction::End).unwrap();
        let remaining: Vec<u64> = center.calls().iter().map(|c| c.id).collect();
        assert_eq!(remaining, vec![second]);
        assert_eq!(center.take_events(), vec![CallEvent::Ended(first)]);
        assert_eq!(center.perform(99, CallAction::End), Err(CallError::UnknownCall(99)));
    }

    #[test]
    fn incoming_prompt_only_for_a_ringing_incoming_call() {
        let mut center = CallCenter::new();
        let incoming = center.report_incoming(phone("5551111")).unwrap();
        let outgoing = center.start_outgoing(phone("5552222")).unwrap();

        let prompt = incoming_prompt(center.call(incoming).unwrap(), false).unwrap();
        assert_eq!(prompt.caller, "5551111");
        assert_eq!(prompt.style, PromptStyle::Banner);
        assert_eq!(prompt.answer, CallAction::Answer);
        assert_eq!(prompt.decline, CallAction::End);
        assert!(incoming_prompt(center.call(outgoing).unwrap(), false).is_none());

        center.respond_incoming(incoming, true).unwrap();
        assert!(incoming_prompt(center.call(incoming).unwrap(), false).is_none());
    }

    #[test]
    fn locked_device_gets_a_full_screen_prompt() {
        let mut center = CallCenter::new();
        let id = center.report_incoming(Handle::Email("ada@example.com".into())).unwrap();
        let prompt = incoming_prompt(center.call(id).unwrap(), true).unwrap();
        assert_eq!(prompt.style, PromptStyle::FullScreen);
        assert_eq!(prompt.caller, "ada@example.com");
    }

    #[test]
    fn declining_ends_the_call_and_answering_connects_it() {
        let mut center = CallCenter::new();
        let declined = center.report_incoming(phone("5551111")).unwrap();
        let answered = center.report_incoming(phone("5552222")).unwrap();
        center.take_events();

        center.respond_incoming(declined, false).unwrap();
        assert!(center.call(declined).is_none());
        assert_eq!(center.take_events(), vec![CallEvent::Ended(declined)]);

        center.respond_incoming(answered, true).unwrap();
        assert_eq!(center.call(answered).unwrap().state, CallState::Connected);
        assert!(center.respond_incoming(answered, true).is_err());
        assert_eq!(center.respond_incoming(99, true), Err(CallError::UnknownCall(99)));
    }
}

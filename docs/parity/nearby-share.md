# Nearby share (parity(nearby-share))

Slice: nearby sharing between BifrOSt machines, the AirDrop-style flow. Matrix rows covered: Multipeer Connectivity
(browse, advertise, advertiser and browser UI, encryption preference) and the Object push transfer UI row.
Not compiled in this round (code and commit only); the lead builds and checks on Midget.

## What exists (before this round)

- Discovery: a private UDP beacon on 224.0.0.251:5354 (`springboard/src/net/share.rs`, `Beacon`). It is not mDNS
  and does not use DNS-SD. See "Not done" below.
- Transfer: TCP with a header MAC, an explicit Accept or Decline on the receiver, a SHA-256 check, and a `.part`
  file renamed on success (`share::handle`, `share::send`).
- Shell: the share sheet with a Nearby row (`springboard/src/shell/share/mod.rs`), the Accept or Decline prompt
  (`shell/share/prompt.rs`), and the service glue (`shell/share/glue.rs`).
- Files: the Files item menu has Share > Send Nearby for files (`springboard/src/files/menu.rs`,
  `files/mod.rs` `Action::SendNearby`), and Photos hands over a picture the same way.

## This round

- Received files land in the person's Downloads folder (`files::fs::downloads()`), no longer in the Files app's
  private `Received` folder. The Files sidebar's Downloads entry uses the same helper. "Save to Files" keeps its
  old destination (`Files/Received`).
- Progress: `net::share::Tracker` records every transfer (sending and receiving) with its bytes done and total,
  and its outcome. The receiver updates it while the body streams in; the sender updates it from `send()`'s
  progress callback.
- Progress card: `springboard/src/shell/share/progress.rs` draws up to three rows under the menu bar: "Sending
  x" or "Receiving x", the peer, "done of total", and a bar. A finished transfer shows "Sent", "Received" or
  "Could not share" with the reason for four seconds, then drops off.
- Sender side: `send_bytes` is removed. Text, contact and file items all go through `share::send` with a
  `Cursor` for in-memory bodies, so every send is tracked. A failure, including a decline, is logged and shown
  on the card.

## Matrix rows

| Row | BifrOSt path | Status after this round | Note |
|---|---|---|---|
| Browsing for nearby services (MCNearbyServiceBrowser) | springboard/src/net/share.rs (`Service`, `Peers`) | Partial, not closed | Discovery is still the private beacon; no app API to browse its own type |
| Advertising a service (MCNearbyServiceAdvertiser) | springboard/sdk/src/nearby_service.rs | Partial, not closed | Not wired into the shell; the beacon is not per-app |
| Advertiser and browser UI (MCAdvertiserAssistant, MCBrowserViewController) | springboard/src/shell/share/mod.rs, prompt.rs, progress.rs | Partial, not closed | The shell now draws the progress card; still no app-facing peer picker |
| Encryption preference (MCEncryptionPreference) | springboard/sdk/src/encryption_preference.rs | Partial, not closed | Body is still plaintext; default secret still "nearby" |
| Object push transfer UI (IOBluetoothObjectPushUIController) | springboard/ui/src/object_push.rs | Partial, not closed | Not touched; the nearby path is LAN only |

No matrix entry is closed by this round. The matrix rows are not edited (the lead does closures).

## Still needs Midget checking

- Two machines on the same LAN: the sheet lists the peer within a few seconds, Accept delivers the file to
  Downloads with a card that moves from 0 to 100 percent, and Decline leaves the sender's card as "Could not
  share ... declined".
- A large file (hundreds of MB) for the card's progress steps and the 30-minute receive deadline.
- Firewall: the beacon uses UDP 5354 and the transfer uses an ephemeral TCP port.

## Not done (open)

- mDNS: the brief asks for discovery over mDNS. The current beacon is a private protocol on 224.0.0.251:5354.
  A real mDNS or DNS-SD responder answers on UDP 5353, which a system responder already binds, so it needs
  either SO_REUSEPORT or a responder-facing design. The DNS-SD name builders in `springboard/core/src/dns_browse.rs`
  are in place for a browse path, but nothing sends or reads those packets. Left for a later round.
- Reference evidence: no REA evidence was used this round. The shipped sharing extension bundles in
  `~/Projects/macos-ref/extract/ref27-index.tsv` were not inspected; the behaviour above comes from the existing
  BifrOSt code.
- The receiver accepts a single file per offer; there is no multi-file offer.

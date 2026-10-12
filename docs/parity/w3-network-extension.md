# Network Extension: wave 3 (DNS settings, relays, URL and content filters)

Wave 3 builds four gap rows of the Network Extension framework as pure Rust models in `springboard/core`. Each model
owns no socket, no thread and no request: the caller passes in a URL or a host name and reads the decision back. None
of them is wired into an app, the shell or the daemons yet, and none has run on the VM or on Midget, so the rows are
`partial`, not `parity`.

## What was built

- `springboard/core/src/dns_settings.rs`: the system-wide encrypted DNS configuration. `DnsProtocol` (DNS over HTTPS
  with a server URL, or DNS over TLS with a TLS server name), `DnsSettings` (server addresses in order, match domains,
  `validate`), `DnsConfiguration` (on or off, description, `set_settings` that keeps the old value on refusal,
  `enable` that needs settings, `resolver_for(host)`). It also holds the suffix rule `domain_matches` and
  `valid_domain` that the other two modules reuse.
- `springboard/core/src/relay.rs`: the relay configuration. `RelayServer` (HTTP/2 URL and HTTP/3 URL, at least one),
  `RelayConfiguration` (one or two relays in order, match and excluded domains, `validate`, `set_enabled`,
  `relays_for(host)`). Excluded domains win over match domains.
- `springboard/core/src/content_filter.rs`: URL filtering. `parse_url`, `sub_urls` (each host suffix crossed with each
  path prefix), `UrlFilter` (rules keyed by host and path prefix, the most specific matching rule decides, `check`
  returns Allow, Block or Undecided) and `flow_verdict` (a flow filter's Allow or Drop; an undecided URL is dropped
  only when `fail_closed` is set; a disabled filter allows everything).

The three `pub mod` lines were added to `springboard/core/src/lib.rs`.

Design rules these models chose themselves (not stated in the official pages read for this wave):

- An empty `match_domains` list means no query or host uses the settings or relays.
- A rule's specificity is the label count of its host, then the segment count of its path.
- An https URL is `https://` followed by a host with no white space. Relay URLs must be https.
- The DNS suffix match and the relay exclusion rule are the same `domain_matches` call.
- Punycode conversion and IPv6 literals are not handled by `parse_url`; a bracketed host returns None.

## Official API names mirrored

Taken from the official DocC pages (`python3 -I tools/parity/docc.py`):

- `NEDNSSettingsManager`: `isEnabled`, `dnsSettings`, `localizedDescription`, `onDemandRules`,
  `loadFromPreferences`, `saveToPreferences`, `removeFromPreferences` (`dns_settings.rs` covers the configuration
  properties; loading and saving to system preferences is left to the caller).
- `NEDNSOverHTTPSSettings`: `serverURL`, `matchDomains`, `init(servers:)`.
- `NEDNSOverTLSSettings`: `serverName`, `matchDomains`, `init(servers:)`.
- `NERelayManager`: `isEnabled`, `relays`, `matchDomains`, `excludedDomains`, `localizedDescription`, `onDemandRules`.
- `NERelay`: `http2RelayURL`, `http3RelayURL` (the other properties of `NERelay` are not mirrored).
- `NEURLFilterManager`: `isEnabled`, `shouldFailClosed`, and the sub-URL enumeration that the page describes as
  generating all possible sub-URLs of a URL before matching. The Bloom filter, PIR server and report properties are
  not mirrored.
- `NEFilterProvider` and `NEFilterNewFlowVerdict`: the Allow and Drop verdicts (`allow()`, `drop()`). Pause, remediate,
  need-rules and URL-append verdicts are not mirrored.

## REA evidence

REA rendered page evidence, run with `tools/parity/rea-page.sh` against the headless Chromium on 127.0.0.1:9333:

- `NEDNSSettingsManager`: `ev_0251b3f1ee4cc8cbf3272a9da54cb1c1267ff066d6a2b143bf77915f93a08fbf`
- `NERelayManager`: `ev_05e3fe819c82f8af8823d53b2ea09f3b66cdde4f8553ff2928d4ff9925590d04`
- `NEURLFilterManager`: `ev_8d128529bb13e1117698c741cb7d4a6b1ae0406619ef7041265d6c84e98c0ff4`

No REA evidence was recorded for `NEFilterProvider` or the other pages read with DocC only.

## Matrix rows closed

These rows are not edited in `docs/parity-matrix/`; the lead updates the matrix centrally.

| Framework | Feature (as written in data/network-extension.json) | New status | BifrOSt path |
|---|---|---|---|
| Network Extension | DNS settings (system-wide encrypted DNS) | partial | springboard/core/src/dns_settings.rs |
| Network Extension | Network relays | partial | springboard/core/src/relay.rs |
| Network Extension | Content filter providers | partial | springboard/core/src/content_filter.rs |
| Network Extension | URL filters | partial | springboard/core/src/content_filter.rs |

Not closed in this wave: Hotspot helper, Wi-Fi accessory setup, DNS proxy provider, local push connectivity, app
extensions host, and the Wi-Fi configuration row (its status is already `os`).

## Tests

Host unit tests are in each module's `#[cfg(test)] mod tests`: 5 in `dns_settings.rs`, 6 in `content_filter.rs`, and 4
in `relay.rs`. They were written by hand from the rules above and have not been run (no builds in this wave).

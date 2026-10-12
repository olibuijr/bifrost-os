# Wave 5: Network Extension (DNS proxy policy, local push)

Branch `parity/w5-network-extension`. Two pure Rust models in the core crate, std only, no unsafe, no hardware access,
no sockets. Nothing is wired into the shell or the SDK yet. Nothing was compiled or run in this wave (no builds were
permitted), so the tests below are written but not executed.

## What was built

- `springboard/core/src/dns_proxy.rs`: the policy a DNS proxy applies to each query.
  - `DnsProxy::new(provider_bundle_id)` starts a proxy that is off and has no rules. `enable`, `disable` and
    `is_enabled` switch it. `add_rule(domain, RuleAction)` adds a rule or replaces the one for the same domain.
    `remove_rule` and `rules` manage the list.
  - `RuleAction` is `Forward`, `Answer(Ipv4Addr)` or `Refuse`. `action_for(name)` returns the action of the most
    specific covering rule (most labels in the domain), using the suffix rule of `dns_settings::domain_matches`.
  - `DnsProxy::decide(query_bytes)` reads one DNS query and returns `Decision::Forward` (the caller sends the query on
    unchanged) or `Decision::Reply(bytes)` (a complete response with the query's id). `Refuse` answers with response code
    3 and no records. `Answer` gives one A record with a 60 s TTL to an A query and an empty NOERROR response to other
    types. A disabled proxy, a query with two or more questions, and any name with no rule are forwarded. A response
    message or unreadable bytes are refused with `ProxyError`.
  - The model sends nothing and reads no traffic. The proxy service owns the socket, sends the reply and forwards.
- `springboard/core/src/local_push.rs`: which push provider runs on which Wi-Fi networks, and the inbox for its messages.
  - `PushManager::new(id, description, provider_bundle_id)` holds the provider identifier, the description, a
    `provider_configuration` map, the enabled flag and the SSID list. `set_match_ssids` checks each name (1 to 32 bytes,
    listed once) and changes nothing when a name is refused. `is_active_on(current_ssid)` is true only when the manager is
    enabled and the current network is listed; no current network means idle.
  - `PushRegistry` keeps managers in insertion order, refuses a duplicate id, and `active_on(ssid)` lists the managers
    that run on a network.
  - `PushInbox::new(capacity)` queues messages in arrival order. It drops the oldest waiting message when full (counted by
    `dropped`), and it drops a message whose id is among the last `capacity` accepted ids (`Accepted::Duplicate`).
- `springboard/core/src/lib.rs`: two `pub mod` lines.

Decisions the official pages do not settle, recorded here so the service layer can change them:
- A proxy that is switched off forwards everything, and a query with no rule is forwarded. The official pages describe
  the proxy as a provider the administrator installs, not what happens to an unmatched query.
- The rule for most specific match is ours. The official pages do not say how overlapping rules resolve.
- The response code is set by patching the low nibble of the second flags byte after `dns_message::encode`. The encoder
  has no response-code field, and adding one would change every existing `Message` literal.
- The inbox size and its duplicate memory (the last `capacity` ids) are ours. The official pages say only that the
  provider keeps a persistent connection and delivers messages; they give no queue size or dedupe rule.
- The SSID limit of 32 bytes is the Wi-Fi network name limit, the same rule `springboard/sdk/src/hotspot_configuration.rs`
  applies. The official pages name the SSID list but do not give the limit.
- Local push does not run a provider or a connection. Reconnect timing and the call and alert path are not modelled.

## Official API names mirrored

- NEDNSProxyProvider (`springboard/core/src/dns_proxy.rs`): the DNS proxy the system runs for each DNS flow on UDP or TCP
  port 53. The model covers the decision for one query. The flow objects, `startProxy`, `stopProxy`,
  `cancelProxyWithError`, `handleNewFlow` and `handleNewUDPFlow` are not modelled, because they need the extension
  host.
- NEDNSProxyManager (`springboard/core/src/dns_proxy.rs`): `isEnabled`, `localizedDescription` (the provider bundle
  identifier and rules stand in), and the provider identity through `providerProtocol`. `shared`,
  `loadFromPreferences`, `saveToPreferences`, `removeFromPreferences` are for the caller's preferences store and are not
  modelled.
- NEAppPushManager (`springboard/core/src/local_push.rs`): `matchSSIDs`, `isEnabled`, `isActive`,
  `localizedDescription`, `providerBundleIdentifier`, `providerConfiguration`. `matchPrivateLTENetworks`,
  `matchEthernet`, `matchMissionCriticalService`, the delegate, and the preference load and save calls are not modelled.
- NEAppPushProvider (`springboard/core/src/local_push.rs`): the receiving side. `PushInbox` is the queue the provider's
  messages go into. The provider's connection methods are not modelled.

## Official references (rendered page evidence)

- Local push connectivity collection group: `https://developer.apple.com/documentation/networkextension/local-push-connectivity`
  REA evidence `ev_0389b030ac45ccdb0f3e579d050b8723505b86efbd3be5611bfa54e5726424fa`.
  States the entitlement requirement, that the push provider extension acts as a local replacement for APNs, and that
  `NEAppPushManager` names the Wi-Fi networks the extension runs on.
- NEDNSProxyProvider: `https://developer.apple.com/documentation/networkextension/nednsproxyprovider`
  REA evidence `ev_d7c98ed02be174ac86276788143184706e1493c55a2fc919a96ce3f94e8bed22`.
  States that the proxy receives DNS flows on UDP and TCP port 53, that it acts as a transparent proxy, and that it
  needs the DNS proxy capability.
- Docc (`python3 -I tools/parity/docc.py networkextension/neapppushmanager`, `.../nednsproxymanager`,
  `.../nednsproxyprovider`, `.../local-push-connectivity`) gave the member lists above.
- The `networkextension/neappushprovider` and `networkextension/dns-proxy` docc paths return 404; the members were taken
  from the rendered pages instead.

## Matrix rows closed

| Framework | Feature (as in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| Network Extension | DNS proxy provider | partial | springboard/core/src/dns_proxy.rs |
| Network Extension | Local push connectivity | partial | springboard/core/src/local_push.rs |

Both rows were `gap` before this wave. Each stays `partial` because no app, shell or extension host uses the model.

## Tests

`#[cfg(test)] mod tests` in each file. Expected values were worked out by hand from the rules above:
- `dns_proxy`: 7 tests. The refused reply is `0x8003` in flags with the query id echoed. The most specific rule wins.
  The label boundary (`badexample.com`) is forwarded. An Answer rule gives its address to A queries and no data to others.
  A disabled proxy and a two-question query are forwarded. Responses and short bytes are refused. Rules replace by domain.
- `local_push`: 9 tests. SSID length (0 and 33 bytes refused, 32 accepted), duplicate names, a refused list keeps the
  old one, activity needs enabled plus a listed network, registry order, duplicate ids, empty fields, inbox duplicates,
  inbox eviction order, and an id leaving the memory being queued again.

Commands to run (not run in this wave): `cargo test -p <core crate> dns_proxy local_push` in `springboard/`.

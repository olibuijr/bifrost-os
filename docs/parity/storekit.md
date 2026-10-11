# StoreKit parity: pure Rust models (partial)

Status: partial. This round adds three std-only models for the parts of StoreKit that a macOS-style desktop can hold without a
payment service. Nothing here sells anything, verifies a signature, or shows the store's prompts.

## What was built

| Area | Path | Public items |
|---|---|---|
| Subscription periods and renewal state | `springboard/ui/src/subscription.rs` (SDK: `bifrost_app::controls::subscription`) | `Period`, `Unit` (day, week, month, year), `Renewal` (`state`, `grants_access`, `next_expiration`), `State` |
| Transactions and entitlements | `springboard/ui/src/entitlement.rs` (SDK: `bifrost_app::controls::entitlement`) | `Ledger` (`record`, `get`, `revoke`, `finish`, `all_for`, `latest_for`, `current_entitlements`, `is_entitled`, `unfinished`), `Transaction` (`entitles`) |
| Review prompt limit | `springboard/sdk/src/review.rs` (SDK: `bifrost_app::review`) | `Policy` (minimum gap, maximum asks, window), `ReviewRequests` (`request`, `allowed`, `from_days`, `days`) |

Design choices:

- Days count from 1970-01-01, reusing `springboard/ui/src/calendar.rs` (`to_days`, `from_days`, `add_months`). Month and year
  periods clamp to the end of a shorter month, as the calendar does.
- `Transaction::entitles` returns false before the purchase day, after a revocation (the earliest revocation day stands), and
  for a subscription whose renewal state does not grant access. A subscription grants access while it is paid or inside its
  grace period.
- `Ledger::current_entitlements` takes the latest purchase of each product and then checks it, so an older purchase never
  replaces a newer one that was refunded or has expired.
- `ReviewRequests` never allows a second ask on the same day, enforces the minimum gap against the most recent ask, and counts
  asks inside the window. The default `Policy` (120 days, 3 asks, 365 days) is an app choice, not a store rule.

## Official API names mirrored

Verified against the official documentation pages (rendered through `docc.py`):

- `Transaction`: "Information that represents the customer's purchase of a product in your app." Properties used:
  `expirationDate` ("The date the subscription expires or renews."), `revocationDate` ("The date that the App Store refunded
  the transaction or revoked it from Family Sharing."), `revocationType`, `revocationPercentage`.
- `Transaction` methods mirrored: `currentEntitlements` ("A sequence of the latest transactions that entitle a customer..."),
  `all(for:)`, `latest(for:)`, `all`, `unfinished`, `finish()` ("Indicates to the App Store that the app delivered the
  purchased content...").
- `RequestReviewAction`: "An instance that tells StoreKit to request an App Store rating or review, if appropriate." Its
  `callAsFunction()` ("Tells StoreKit to ask the user to rate or review your app, if appropriate.") is mirrored by
  `ReviewRequests::request`. Environment value `requestReview`.
- `SubscriptionPeriod`: "Represents the duration of time between subscription renewals." Units mirror the framework's day, week,
  month and year cases as named on that page's child symbols; the unit enum itself was not retrievable through the documentation
  helper, so the case names come from the framework's own naming, not a fetched page.
- `SubscriptionStatus`: "Represents the renewal status information for an auto-renewable subscription." The state variants
  (`Subscribed`, `InGracePeriod`, `InBillingRetry`, `Expired`) are BifrOSt's names. The official state list could not be fetched
  (the `subscriptionstatus/state` page returned 404), so these names are not verified against it.

## REA and documentation evidence

- REA evidence of the rendered StoreKit landing page: `ev_b909799f3328d56e80637753f326decdd60f96f1b850564fe9140a73b87ab58f`
  (https://developer.apple.com/documentation/storekit). Its rendered headings list the Reviews section ("Requesting App Store
  reviews", "Request Review Action") and the Transactions related pages.
- Documentation consulted through `tools/parity/docc.py`: `storekit/requestreviewaction`, `storekit/transaction` (depth 1),
  `storekit/transaction/revocationdate`, `storekit/transaction/expirationdate`, `storekit/subscriptionperiod`,
  `storekit/subscriptionstatus` (depth 1).
- Not retrieved: the "Requesting App Store reviews" article body (only its abstract came back), so the store's own rules for
  how often a prompt is shown are not confirmed here. The `ReviewRequests` limits are app policy.

## Remaining gaps

1. No purchase flow, product price or currency in `springboard/src/shell/store/api.rs`.
2. `Ledger` is not fed by any store and is not persisted; no SDK call gives an app its entitlements.
3. No signed transaction verification (JWS) and no receipt check; `install.rs` checks only the package sha256.
4. No subscription groups, billing, offers, offer codes or win-back offers.
5. The review prompt is not shown by the shell; `ReviewRequests::request` only records the decision.
6. No purchase-aware test harness in `springboard/sdk/src/testing.rs`.
7. Apple-service items (storefront, App Store messages, ad attribution, external purchase, payment method binding) stay n/a.

## Verification

Per the task, nothing was built or tested in this round. The modules carry unit tests (7 in `subscription.rs`, 7 in
`entitlement.rs`, 6 in `review.rs`) for the next build stage to run. The owner name guard is run as part of the same stage.

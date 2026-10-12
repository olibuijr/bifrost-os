# StoreKit parity, wave 3: purchase flow, offers, app transaction

Three gap rows of the StoreKit framework are built as pure Rust models. Each model holds the rules and the
state; nothing charges a customer, signs a record or reads a file. The store and the installer call them later.
Wave 1 already holds the transaction ledger, subscription renewal and review-prompt policy (`entitlement.rs`,
`subscription.rs`, `review.rs`), so this wave does not repeat them.

## What was built

| Module | Mirrors (official API) | What it does |
|---|---|---|
| `springboard/ui/src/purchase.rs` | `Product.purchase(options:)`, `Product.PurchaseResult` (`success`, `userCancelled`, `pending`), `Product.PurchaseAction` | `Price` (minor units, fraction digits, display text). `Purchase` steps idle, confirmation shown, pending, finished, and refuses out-of-order results. `may_install()` allows a paid install only after a successful purchase. |
| `springboard/ui/src/offer.rs` | `Product.SubscriptionOffer`, `Product.SubscriptionOffer.OfferType` (`introductory`, `promotional`, `winBack`) | `Offer` with kind, price, period and period count, and `ends_on`. `Customer` facts decide `applies`: introductory for a new customer, win-back inside a day window after the lapse, promotional for anyone. `choose` returns the first applicable offer in the app's order. |
| `springboard/sdk/src/app_transaction.rs` | `AppTransaction` (`appTransactionID`, `environment`, `bundleID`, `appVersion`, `originalAppVersion`, `originalPurchaseDate`, `revocationDate`) | Record of how the customer got the app. `updated_since_purchase`, earliest-wins `revoke` and `is_revoked_on`, package digest check, and a tab-separated line format for the installer's file. |

## Not built, and why

- `Product.PaymentMode` (the offer payment modes): the official page could not be read through
  `tools/parity/docc.py` (HTTP 404 for `storekit/product/subscriptionoffer/paymentmode`), so its cases are not
  mirrored. The offer model therefore has no payment mode and no total price.
- Offer codes (redemption) and the signed promotional offer signature: these need store-signed data, which BifrOSt
  does not have. Only the offer types and eligibility are modelled.
- Signed app transaction (`AppTransaction.shared`, `deviceVerification`, `signedDate`): no signature is
  checked. `verifies_package` compares a SHA-256 digest only, which is the check the installer already makes.
- The purchase sheet, paid-app install path and store catalogue prices: `springboard/src/shell/store/` is not
  changed in this wave. `Price` is not yet read from `StoreApp`.
- Nothing is wired into apps or the shell, and nothing has been tested on the VM or Midget.

## Official API evidence

Apple facts were read with `python3 -I tools/parity/docc.py` and checked against the rendered pages.

- `storekit/product/purchase(options:)`: "Initiates a purchase for the product with the App Store and displays the
  confirmation sheet." Returns `Product.PurchaseResult`.
- `storekit/product/purchaseresult`: `success(_:)` "results in a transaction", `userCancelled`, `pending`
  "requires action from the customer".
- `storekit/product/subscriptionoffer/offertype`: `introductory`, `promotional`, `winBack`.
- `storekit/apptransaction`: "Information that represents the customer's purchase of the app, cryptographically
  signed by the App Store." Fields listed above.

REA evidence (rendered official pages, `tools/parity/rea-page.sh`):

- `ev_06cd2005378b16bf8a25d1583681134a8eeef66bc900797e33ea699128465839`: StoreKit landing page.
- `ev_2db215229464a956491cab0cf0717423efc503cfee6edd1c08aeb3490624e350`: `purchase(options:)`.
- `ev_d8bdda73c24f73ee5036123102f71acf0129f6170cb4aecfa304135b293ca8c9`: `Product.SubscriptionOffer`.
- `ev_3e67459cbd8baec11a9b4cc762567d6e558a73240ef49f2fe46f0cc6eb71f32b`: `AppTransaction`.

## Matrix rows closed

These rows go from `gap` to `partial` in `docs/parity-matrix/data/storekit.json`. The lead updates the matrix.

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| storekit | Purchase flow | partial | springboard/ui/src/purchase.rs |
| storekit | Offers, offer codes and win-back offers | partial | springboard/ui/src/offer.rs |
| storekit | App transaction (purchase of the app itself) | partial | springboard/sdk/src/app_transaction.rs |

## Tests

Each new module has a `#[cfg(test)] mod tests` with hand-worked values:

- `purchase.rs`: 12.50 EUR, 990 ISK, -0.75 USD, six-digit clamp, the sheet-before-result rule, cancelled and
  pending flows.
- `offer.rs`: three weekly periods from day 10 end on day 31; introductory offer for new and returning customers;
  promotional for everyone; win-back on day 130 and not on day 131 after a lapse on day 100; the order `choose` uses.
- `app_transaction.rs`: the exact eight-field line, round trip with a revocation day, malformed lines, a tab in a
  field, the version comparison, earliest revocation, and the case-insensitive digest.

No build or test was run in this round, as the wave rules require.

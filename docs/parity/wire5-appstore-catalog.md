# Wire 5: App Store catalogue, product metadata and Purchased (storekit rows)

The shell's App Store (`springboard/src/shell/store/`) now shows what the catalogue says a product costs and how old
it is rated, and lists the apps the customer has purchased from the entitlement ledger. The Updates tab with version
notes and the Version History page were already wired by earlier rounds and are re-checked here, not rebuilt.

## What is now wired

| Piece | Model reused | BifrOSt path |
|---|---|---|
| Price and currency on each catalogue entry (`price` in minor units, `currency`, optional `fraction_digits`, default 2). An entry with no currency is free. | `bifrost_ui::purchase::Price` (`display()` gives "12.50 EUR") | `springboard/src/shell/store/api.rs` (`StoreApp::price`, `is_paid`, `price_text`, `parse_price`) |
| Age rating (`age_rating`, one of 4+, 9+, 12+, 16+, 18+; anything else is unrated and shows "-") | none existed; small pure check in `api.rs` (`AGE_RATINGS`, `age_rating_of`) | `springboard/src/shell/store/api.rs` |
| Product page metadata row: VERSION, SIZE, PRICE, AGE, CATEGORY | the existing product page | `springboard/src/shell/store/mod.rs` (`draw_page`) |
| Purchased list: installed apps that the entitlement ledger entitles on today, newest purchase first, under the Updates tab header "Purchased" (replaces "Installed") | `bifrost_ui::entitlement::Ledger` (`record`, `current_entitlements`) | `mod.rs` (`AppStore::purchased`), `install.rs` (registry field `purchased`) |
| First install day kept in the app registry (`apps/installed.json`, field `purchased`, days since 1970-01-01; written only when known, kept across updates, 0 for old registries) | `InstalledApp::purchased`, `install_day()` | `springboard/src/shell/store/install.rs` |
| A paid app is never installed by GET: the shell has no purchase sheet, so the tap sets "Purchase is not available yet" | `StoreApp::is_paid` | `mod.rs` (`press_button`) |

Already present and re-checked: the Updates tab lists each pending update with its version notes (two lines, then
"more"), the product page shows "What's New" and "Version History" (every release, newest first), and Roll Back.

## How a user reaches it

- Open the App Store from the Dock. Tap an app on Today, Apps or Games: the product page shows VERSION, SIZE, PRICE
  ("Free" for every app the catalogue prices at zero), AGE (or "-") and CATEGORY.
- Updates tab: "Available Updates" (with version notes), then "Purchased": every installed app, newest purchase first.

## How to verify on the test PC (Midget)

1. `bifrost pc midget state` shows the store running; open it with `bifrost pc midget open-app <store id>` or from the
   Dock.
2. Apps tab, tap a free app: the product page row reads PRICE "Free" and AGE "-" (the catalogue does not rate apps
   yet). Compare with `bifrost pc midget screen`.
3. Updates tab: after an install, the app appears under "Purchased" directly below "Available Updates", and the
   newest install is first. An app installed before this change (no `purchased` field) sorts last by name.
4. A paid entry (catalogue `price` above zero with a `currency`) shows its price on the product page; tapping its GET
   shows "Purchase is not available yet" and downloads nothing. No paid entry exists in the live catalogue yet.

## REA evidence (rendered official pages, `tools/parity/rea-page.sh`)

- `ev_a66b5b08f9fbdcb6ee30582d31b04855de17c5dbcca53d0881583330dda3627c`: `Product` (price, displayPrice, purchase, currentEntitlements list)
- `ev_3db769860a48e47fe310b6b4f6e0e145b0939b4dec0331c365bae4f98ef7f675`: `Transaction.currentEntitlements` (one transaction per product that entitles the customer; refunded or revoked products are left out)
- `ev_617dc064d14405470690c8f364b5994725af5e5966bb4df62059c9e972389438`: `Product.displayPrice` (localized display string; use `price` for arithmetic)

## Tests (hand-worked, not run in this round)

- `api.rs` `price_and_age_rating_come_from_the_catalogue`: 1250 EUR gives "12.50 EUR"; 990 with 0 digits gives "990 ISK";
  an unknown rating "7+" gives "-"; a price with no currency is free; " 18+ " trims to "18+".
- `mod.rs` `purchased_lists_installed_apps_by_purchase_day_and_hides_services`: on day 200 the apps bought on day 120
  and 100 are entitled, the service is left out, the app bought on day 300 is not yet entitled; rows are App(1), App(0).
- `mod.rs` `a_paid_app_is_not_installed_without_a_purchase`: a paid GET starts no job and records the message.

## Remaining gaps

- No purchase: no payment sheet, no purchase result, no paid-app install path. `springboard/ui/src/purchase.rs` is
  still unwired (a paid entry is refused, not bought). Matrix row "Purchase flow" stays partial.
- Transactions are not real store transactions: the Purchased list uses each installed app as a free purchase on its
  first install day. No refunds, no revocations, no transaction history from the server.
- Subscriptions, offers and offer codes (`ui/src/subscription.rs`, `ui/src/offer.rs`) are not in the catalogue and are not shown.
- Price is shown as the catalogue gives it; no localized currency formatting beyond `Price::display`.
- Age ratings are a plain check; no questionnaire and no content descriptors.
- The product page's "Overlay" (embedded product sheet for another app) is not built.

## Matrix rows

| Slug | Feature (as written in docs/parity-matrix/data/storekit.json) | BifrOSt path |
|---|---|---|
| storekit | Product catalogue and metadata | springboard/src/shell/store/api.rs |
| storekit | App Store product page and overlay | springboard/src/shell/store/mod.rs |
| storekit | Transactions and entitlements | springboard/src/shell/store/mod.rs (AppStore::purchased), springboard/ui/src/entitlement.rs |
| storekit | Store catalogue install and update | springboard/src/shell/store/install.rs |
| storekit | Purchase flow | springboard/src/shell/store/mod.rs (press_button refuses paid apps); springboard/ui/src/purchase.rs not yet wired |

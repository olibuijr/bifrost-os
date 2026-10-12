# Wire 6: App Store purchase sheet and Restore Purchases (storekit rows)

The shell's App Store (`springboard/src/shell/store/`) now sells a paid app. GET on a paid catalogue entry opens a purchase
sheet with the app, its price and Buy or Cancel. A confirmed purchase is recorded in the entitlement ledger and only then is
the paid package installed. Restore Purchases re-reads the purchases of this PC. The model is `bifrost_ui::purchase`
(`Purchase`, `Outcome`, `Step`), and the ledger is `bifrost_ui::entitlement::Ledger`; both were already built and are reused
unchanged.

## What is now wired

| Piece | Model reused | BifrOSt path |
|---|---|---|
| Purchase sheet: app name, price (`Price::display`), Buy and Cancel, drawn over the pane | `purchase::Purchase` (`begin`, `complete`, `may_install`) | `springboard/src/shell/store/sheet.rs` (`Sheet`, `sheet_card`, `sheet_buttons`), `mod.rs` (`draw_sheet`, `open_sheet`) |
| Passcode confirmation: with a passcode set, Buy opens the shell keypad; the code is checked by the passcode store (wrong codes count, lockouts apply) | `passcode::Store`, `passcode::Keypad` | `sheet.rs` (`buy`, `check`) |
| Records the transaction on confirmation, then installs the paid package | `entitlement::Ledger::record` | `mod.rs` (`finish_purchase`, field `bought`, merged in `purchased`) |
| Restore Purchases row at the foot of the Updates tab; reports "Restored N purchases." or "No purchases to restore." | the install registry (`install::installed_in`) and `AppStore::purchased` | `mod.rs` (`restore_purchases`, `Row::Restore`, `Target::Restore`) |

Cancel, a wrong passcode and a lockout record nothing and install nothing. A sheet touch never reaches the store's rows
underneath it.

## How a user reaches it

- Open the App Store from the Dock, tap a paid app (Today, Apps or Games), then GET. The sheet opens; nothing downloads.
- Buy confirms (after the passcode when one is set). The install then runs as for a free app, and the app appears under
  "Purchased" on the Updates tab.
- Updates tab, last row: "Restore Purchases".

## How to verify on the test PC (Midget)

1. Without a paid catalogue entry nothing changes on the live store (none exists yet). Check the Updates tab shows
   "Restore Purchases" under "Purchased" and tapping it reads "No purchases to restore." on a fresh PC, or "Restored N
   purchases." when installs exist.
2. To exercise the sheet, a catalogue entry with `price` above zero and a `currency` is needed (the catalogue is served
   by the store). Tap its GET: the sheet shows the name and price; Cancel closes it and `apps/` gets no new package.
3. With a passcode set (Settings > Passcode), Buy shows the keypad titled "Enter Passcode" with the message "Buy <name> for
   <price>". A wrong code shakes and reads "Wrong passcode"; the right code installs the app.
4. Compare with `bifrost pc midget screen` and the Dark and Light appearances (`bifrost pc midget appearance`).

## Tests (hand-worked, not run in this round)

- `sheet.rs`: Buy without a passcode confirms at once; with a passcode (`1234` stored) Buy shows the keypad with the message
  "Buy Chess for 990 ISK", and the entry 1234 confirms; entry 9999 gives "Wrong passcode" with the entry cleared (one failure,
  no lockout); the Buy and Cancel rectangles lie inside the card and do not overlap.
- `mod.rs` `a_paid_app_opens_the_purchase_sheet_and_installs_nothing_yet`: GET opens the sheet, starts no job, price "990 ISK".
- `mod.rs` `tapping_buy_records_the_purchase_and_cancel_records_nothing`: a tap on Cancel closes the sheet and the ledger has
  no entry; a second GET and a tap on Buy record the purchase (entitled today) and close the sheet. The test store has no
  worker pool, so no download starts.
- `mod.rs` `restore_purchases_reports_the_count_on_the_updates_tab`: the Restore row is on the Updates tab; activating it on an
  empty store reads "No purchases to restore.".
- `mod.rs` `purchased_lists_installed_apps_by_purchase_day_and_hides_services` now reads rows `at + 1..at + 3` and expects
  `Row::Restore` at `at + 3`.

## REA evidence (rendered official pages, `tools/parity/rea-page.sh`)

- `ev_f6696dc705f599aca45812342c99ffda1e21771ff246279c5177460855154bcd`: `Product.purchase(options:)` (initiates the purchase
  and displays the confirmation sheet; the sheet is the customer's confirm or cancel; the call may throw a purchase error).
- `ev_a66b5b08f9fbdcb6ee30582d31b04855de17c5dbcca53d0881583330dda3627c`: `Product` (price, purchase, currentEntitlements).
- `ev_3db769860a48e47fe310b6b4f6e0e145b0939b4dec0331c365bae4f98ef7f675`: `Transaction.currentEntitlements` (refunded or revoked
  products are left out).

## Remaining gaps

- No payment. The sheet records the purchase in the local ledger only. There is no store-side charge, no pending purchase
  (`Outcome::Pending` is not produced), no Ask to Buy and no external purchase.
- The ledger is in memory for the session. After a restart a paid app's entitlement comes from its install record (the
  `purchased` day in `apps/installed.json`); the purchase price and transaction id are not kept.
- Subscriptions list: not wired. The catalogue (`api.rs`) carries no subscription fields and the cache format has none, so
  `springboard/ui/src/subscription.rs` (`Renewal`, `State`) has no data to show. It needs a catalogue schema change first.
- Refunds and revocations: not wired. `Ledger::revoke` has no source; nothing in the store refunds a purchase.
- Transaction history beyond the Purchased list: not wired.
- Passcode path: the sheet reads `root/passcode.ini` where `root` is the store root (`/var/lib/springboard`), the same
  directory Settings uses; a PC without that file (no passcode) confirms Buy at once.
- Price on the GET button: the button still reads GET; the price shows on the product page and in the sheet.

## Matrix rows

| Slug | Feature (as written in docs/parity-matrix/data/storekit.json) | BifrOSt path |
|---|---|---|
| storekit | Purchase flow | springboard/src/shell/store/sheet.rs; springboard/src/shell/store/mod.rs (open_sheet, finish_purchase) |
| storekit | Transactions and entitlements | springboard/src/shell/store/mod.rs (finish_purchase, purchased, restore_purchases); springboard/ui/src/entitlement.rs |
| storekit | Product catalogue and metadata | springboard/src/shell/store/api.rs (price used by the sheet) |
| storekit | Store catalogue install and update | springboard/src/shell/store/install.rs (paid package installed after a confirmed purchase) |

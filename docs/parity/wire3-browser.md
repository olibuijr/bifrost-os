# Wire 3: Browser app (`springboard/apps/browser`, 0.11.0)

Third wiring round for the Browser. The round had three leftovers: the Reader view built from the DOM node snapshot,
password AutoFill under login fields, and a Share menu item. Two of the three are wired. AutoFill is not, for the
reasons under "Remaining gaps". The branch was written without builds (no cargo, rustc or tests), so the lead's build is
the first compile.

## What is now wired

| Piece | Model | Where in the app |
|---|---|---|
| Reader page rebuilt from a DOM node snapshot | `springboard/ui/src/dom_snapshot.rs` (`DomSnapshot`, `NodeRecord`, `NodeKind`) | `reader.rs`: the in-page script reports the article as flat records (`id`, `parent`, `kind`, `name`, `text`, `attributes`); `parse_nodes` turns them into `NodeRecord`s, `DomSnapshot::new` validates the tree, and `render_body` writes the body from it. Only the allowed tags pass, text is escaped, and only `href`, `src` and `alt` survive. The old path that copied the page's `innerHTML` is gone. |
| View > Show Reader | none (app menu item) | `main.rs`: `App::main_menu` lists a View menu with "Show Reader" (key Cmd+Shift+R, check mark while the reader page is up, greyed out when the page has no article). The shell merges it into its own View menu. `App::menu_action` runs `toggle_reader`. |
| File > Share | none (adapter) | `main.rs`: `App::main_menu` lists a File menu with "Share…", enabled on http and https pages. `share_page` sends `Payload::Url(address)` to `bifrost_app::share::show_picker`, which is the shell's share menu (Copy and the apps that take links). |
| Content area colour | none | `main.rs` `draw` fills with `CONTENT_BG` (white in Light, dark grey in Dark), not the pure black `DETAIL_BG`. |

### Why the Share sheet is the system picker, not the compose model

`springboard/ui/src/compose.rs` (`ComposeSheet`) and `springboard/sdk/src/social.rs` model the Social framework's compose
post sheet. Apple's own documentation says a compose controller posts to a social service the user has set up, and
the matrix row "Compose post sheet" is `partial` for the same reason: BifrOSt has no social-network service and no
social account store. A compose sheet with nothing to post to would be a dead control. The Share item therefore uses the
shell's share picker, which is the share sheet the reference platform presents for a URL (see REA evidence below).

## How a user reaches it

- **Show Reader:** the View menu, or Cmd+Shift+R. The item is enabled when the page has an article (the same test that
  shows the Reader button in the address field). It is checked while the reader page is up, and choosing it again, Esc
  or the button returns to the original page.
- **Share:** the File menu, "Share…". The shell's share menu opens with the page address. Choosing Copy puts the address
  on the shared pasteboard.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Open a long article (for example a long news page). Open the View menu: "Show Reader" is enabled. Choose it. The
   article shows with the title, byline and body in a 680 px column, and the menu item has a check mark.
3. Open a page with no article (a search page). View > Show Reader is greyed out.
4. Choose View > Show Reader again, or press Esc: the original page returns.
5. Check the reader page's markup in the Develop tools (Cmd+Option+U shows source of the original page only). A reader
   page should contain no `<script>`, `<form>` or `<nav>` elements.
6. Open File > Share on any http page: the share menu opens with the page address. Choose Copy, then paste into a text
   field to confirm the address.
7. Take a Midget screenshot of the Browser with `bifrost pc midget screen PATH` and check it with the bifrost-hig checklist
   (menu items in standard menus, content area in the window's content colour, Light and Dark via
   `bifrost pc midget appearance`).

Unit tests (not run here; the lead runs them): `reader::tests::article_is_rebuilt_from_the_node_snapshot`,
`reader::tests::empty_or_malformed_snapshots_are_no_article`, and the existing `reader::tests::data_url_is_encoded_and_page_follows_appearance`.
Expected values were worked out by hand: `<p>a < b</p>` with a script sibling and a link with `href="https://e.com/?a=1&b=2"`
renders as `<p>a &lt; b</p><a href="https://e.com/?a=1&amp;b=2"></a>`.

## REA evidence

- `ev_09a054db0b498f13c3ff44f95e0e96e3806450cf5f1ab4aad9ea986181455d65`: official page "SLComposeViewController" (rendered
  with `tools/parity/rea-page.sh`). It says the compose controller is used to post to a social service, that the caller
  checks `isAvailable(forServiceType:)` first, and that the content must be set before presenting it. This is why the
  Share item does not use the compose model.
- `ev_0465bc0d39d9dad38a96d9bd2d08518fd6ed1e217cf64fa0b2d0116590caf042`: official page "NSSharingServicePicker" (rendered).
  It says the sharing service picker shows a list of services for one or more items and shares the item with the chosen
  service, presented from an interface with `show(relativeTo:of:preferredEdge:)`. This matches the shell share menu the
  Share item opens.
- Symbol text (no evidence id): `python3 -I tools/parity/docc.py` was not run for this round; the rendered pages above
  are the references.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status after this round |
|---|---|---|---|
| webkit | DOM node snapshot | `springboard/ui/src/dom_snapshot.rs` (used by `springboard/apps/browser/src/reader.rs`) | still `partial` (the Browser is the first consumer; no app-facing DOM snapshot API) |
| link-presentation | Sharing a URL with metadata to the share sheet | `springboard/apps/browser/src/main.rs` (`share_page`, through `bifrost_app::share::show_picker`) | still `partial` (the URL goes as plain text; no link preview) |
| social | Compose post sheet | `springboard/ui/src/compose.rs`, `springboard/sdk/src/social.rs` | not wired (no social service) |
| authentication-services | AutoFill credential provider | `springboard/ui/src/credential_identity.rs` | not wired (see gaps) |

## Remaining gaps

- **Password AutoFill (not built):** the Browser keeps no saved logins, so there is nothing to suggest. Wiring it needs
  (a) a saved-credential store: `IdentityStore` in memory is not enough, it needs the keychain service
  (`docs/parity/wire-svc-keychain.md`, manifest `permissions = ["keychain"]`, asynchronous replies); (b) a save step when a
  form with a password field is submitted; (c) a login-field check from the page (an in-page script that reports
  `input[type=password]` and its rectangle); (d) a suggestion popup over the web view and a fill script that sets the
  field value and dispatches `input` and `change`. Each part is a separate change, and none could be built or run here.
- **Share anchor:** the picker is anchored at a zero rectangle at the window's top-left, because the menu does not
  know where the address field is drawn. The shell positions the picker from that point, so it may not sit under the
  address field.
- **Share targets:** only the apps that declare `handles` for `public.url` or a matching scheme appear, as the shell
  decides. Share extensions are not available (matrix row "ExtensionKit" stays partial).
- **Reader:** the reader page keeps the article's text and the allowed tags only. Tables, images and lists come through;
  video, forms and iframes do not. Articles split across several elements can be picked as one block only when the
  scoring picks their container, as before.
- **Not built or run:** no cargo, rustc or tests were run in this round, per the wiring rules. The matrix data is not
  edited here; the lead updates `docs/parity-matrix/` after the build.

# Theme 16: action extension view controller

Theme: "Action extension view controller" (size 28, 17 frameworks). This wave builds the shared request model
that an action extension's view controller works through, and closes the two File Provider UI rows that depend on it.
The other rows in the theme (compose sheets, pickers, notification list views, route pickers, finder sync) need
their own hosts and are not closed here.

## What was built

- `springboard/ui/src/actionrequest.rs` (extended). The `ActionRequest` that a chosen file action runs through now
  covers the File Provider UI context surface:
  - `try_new(action, items)` refuses an item identifier that is empty, holds whitespace or control characters, or
    appears twice in the selection. `new` is unchanged and does not check.
  - `in_domain(domain)` and `domain_identifier()` record and read the domain the items belong to. An empty or
    multi-token domain is refused.
  - `show_error`, `complete`, and `cancel(Some(message))` keep their behaviour. A finished request never changes and
    clears the shown error.
  - `RequestError` gains `InvalidItem(String)`.
  - Eight tests in the module (five existing, three new).
- `springboard/sdk/src/extension.rs`. Re-exports `ActionRequest`, `Outcome` and `RequestError` for apps, so an app
  can build and finish a request through the SDK.

## Official API names mirrored

| Apple (File Provider UI) | BifrOSt |
|---|---|
| `FPUIActionExtensionViewController.prepare(forAction:itemIdentifiers:)` | `ActionRequest::try_new` / `new` (action and item identifiers, pending) |
| `FPUIActionExtensionViewController.prepare(forError:)` | `ActionRequest::show_error` (error shown, request stays pending) |
| `FPUIActionExtensionContext.completeRequest()` | `ActionRequest::complete` |
| `FPUIActionExtensionContext.cancelRequest(withError:)` | `ActionRequest::cancel(Some(message))`; `cancel(None)` records a user cancel |
| `FPUIActionExtensionContext.domainIdentifier` | `ActionRequest::domain_identifier` / `in_domain` |

Apple's `extensionContext` (the object the host hands the view controller) is not modelled as a property; the request
object is what the hosted view receives and finishes.

## REA evidence

- `ev_0ed422ae129a65f7a14bcaa0aafd9a1a3e73d61e0a31c9f5c3d8bcf5689a1a79`: FPUIActionExtensionViewController
  (https://developer.apple.com/documentation/fileproviderui/fpuiactionextensionviewcontroller). Source of the
  "subclass once, call prepare, complete or cancel" lifecycle.
- `ev_57af2731422af772d5b5177c85bd5478560ebce3ac4f3306eaa68b1d333d9303`: FPUIActionExtensionContext
  (https://developer.apple.com/documentation/fileproviderui/fpuiactionextensioncontext). Source of completeRequest,
  cancelRequest(withError:) and domainIdentifier.

DocC (`tools/parity/docc.py`) was read for both symbols and agrees with the rendered pages.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| file-provider-ui | Action extension view controller | partial | springboard/ui/src/actionrequest.rs; springboard/sdk/src/extension.rs |
| file-provider-ui | Action extension context: complete or cancel | partial | springboard/ui/src/actionrequest.rs (complete, cancel, in_domain) |

Both rows stay partial. The request model exists, but no extension process runs the view controller, and the Files
browser does not offer the action. Those are the gaps in the existing rows' notes: "there is no extension process that
the shell hosts" and the context menu for Files actions.

## Not closed, and why

- File Provider UI "Authentication error handling in actions" (`prepare(forError:)` for cloud sign-in): n/a. BifrOSt
  cloud accounts are not a File Provider domain. `show_error` covers only the in-view error message.
- Other theme-16 rows (Social compose sheet, Contacts UI picker, ScreenCaptureKit, ReplayKit, AVRouting, Notification
  Center list, Finder Sync, ExtensionKit enablement, PermissionKit button): each needs a host surface this wave does
  not build.

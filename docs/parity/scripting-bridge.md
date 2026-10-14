# Scripting bridge: attaching to a running app

## What it mirrors

Attaching to a running scriptable app by bundle identifier, process id or bundle URL, and the per-app control socket
that the app's scripting calls travel over.

## What is wired

- `springboard/sdk/src/script_target.rs` (`bifrost_app::script_target`): `Locator`, `RunningApp`, `attach` and
  `socket_path`. `attach` is pure: the caller passes the running-app list. A zero process id and an empty identifier or
  URL are invalid, and a locator that matches nothing is `NotRunning`.
- `socket_path` is the rule the SDK runtime uses to bind an app's control socket (`runtime.rs`, `control_endpoint`):
  an explicit path, else `$XDG_RUNTIME_DIR/bifrost/apps/<id>.sock`, else `control.sock` in the data directory.
- Tests: `springboard/sdk/tests/parity_sdk-complete-29.rs`.

## What still needs the lab machine or the shell

- The shell's running-app list carries only app identifiers, so attaching by process id or bundle URL waits for the shell
  to report them.
- There is no handle object that sends calls to an attached app.
- Launch flags and an explicit activate request need a new workspace operation in `proto` and a handler in the shell.

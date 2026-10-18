# Spotlight scope (Settings > Spotlight)

Code: `springboard/src/shell/spotlight_scope.rs` (the scope model), `spotindex.rs` (the Core Search index), the Spotlight
page in `shell/pages.rs` and `settings.rs`.

## What the page controls

- Which kinds of result Search lists. Each category is on by default. The categories are the `Scope` kinds in
  `spotlight_scope.rs`, in the order the page lists them.
- Folders that Search never lists. A folder is excluded together with everything under it.

## Storage

- `settings.ini` holds two keys. `spotlight_off` lists the switched-off categories, comma separated.
  `spotlight_excluded` lists the excluded folders, one escaped field each, joined by tabs.
- The shell keeps the current scope in memory, so Search reads it on each query without touching the disk.
- `Scope::hides` is the one question Search asks: for a hit's category and path, is it hidden.

## Index

Apps send their searchable items to the shell's index (`spotindex.rs`). The Search panel shows the index under
"Documents". The panel ranks matches with `bifrost_ui::search_rank`.

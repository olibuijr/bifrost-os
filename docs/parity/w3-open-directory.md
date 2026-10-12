# Wave 3: Open Directory (record queries, node sessions)

Framework: Open Directory (`docs/parity-matrix/data/open-directory.json`, official doc
https://developer.apple.com/documentation/opendirectory). Two gap rows are built as pure models in `bifrost_ui`.
Nothing here connects to a directory server; a node is a named container that the caller fills.

## What was built

- `springboard/ui/src/directory_node.rs`: `Session` holds a tree of nodes keyed by path (`/Local/Default`).
  A top-level node needs no parent, any other node needs its parent. `subnode_names` lists immediate children in
  name order. A node with subnodes cannot be removed. `Node` keeps its `DirRecord`s unique by (type, name).
  Path rules: absolute, no empty, `.` or `..` components, at most 256 bytes.
- `springboard/ui/src/directory_query.rs`: `DirRecord` (type, name, multi-valued attributes; the attribute `name`
  reads the record name and cannot be set) and `Query` (exact record type, a `recordsearch::Element` tree, the
  attributes an any-attribute match looks at, a sort attribute with records lacking it last, and a result limit).
  The query runs over a slice of records the caller holds, so it never touches a server.
- `springboard/ui/src/lib.rs`: two `pub mod` lines.

## Official API names mirrored (from the official pages)

- Sessions and nodes: `ODSession`, `ODSessionRef`, `ODNode`, `ODNodeRef`, `ODNodeCopySubnodeNames`
  (https://developer.apple.com/documentation/opendirectory, "Open Directory Functions").
- Records and queries: `ODRecord`, `ODRecordRef`, `ODQuery`, `ODQueryRef`, `ODQueryCopyResults`
  (the landing page lists the classes ODRecord and ODQuery; the function names are taken from the matrix row, which
  cites the "Open Directory Functions" reference, and were not re-read one by one on their pages in this wave).
- Result delivery is not built here. The callback and dispatch-queue rows stay with `springboard/sdk/src/dispatch.rs`.

## Limits (stated, not hidden)

- No network: there is no session to a server, no LDAP and no authentication. Node contents are whatever the caller
  adds. `ODRecordVerifyPassword` and policies are not modelled.
- Queries are exact over the records given. There is no paging, no server-side search and no schema mapping.
- `springboard/ui/src/identity.rs` (users and groups) is separate. It was not changed.

## Evidence

- REA rendered page of the framework landing page: `ev_4599c5d4b31685338552279a855717b090773d099b16e72fa63f09f2b87c1436`
  (title "Open Directory | Apple Developer Documentation"; lists the classes ODNode, ODQuery, ODRecord, ODSession and
  the ODQueryDelegate protocol).
- Official DocC: `python3 -I tools/parity/docc.py opendirectory --depth 1` (classes and reference groups above).

## Tests

Each new module has a `#[cfg(test)] mod tests` with hand-worked expectations:
- `directory_node`: parent rule and node uniqueness; path validation; subnode listing order; removal refused while
  subnodes exist; record uniqueness by type and name; record ordering by (type, name).
- `directory_query`: default query keeps input order; type filter; attribute comparison ignoring case; any value of a
  multi-valued attribute counts; any-attribute search limited to the listed attributes; records lacking the sort
  attribute sort last; limit after sort; `name` reads and cannot be set; empty list removes an attribute.
Build and test runs were not done in this wave (no builds by rule).

## Matrix rows closed

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| Open Directory | Directory sessions and node connections | partial | springboard/ui/src/directory_node.rs |
| Open Directory | Record queries and search results | partial | springboard/ui/src/directory_query.rs |

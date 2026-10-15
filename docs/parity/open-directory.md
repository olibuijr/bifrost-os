# Open directory contact search

Mirrors the contact search of a directory service: a query over the person entries of a directory node, and the
same query as an LDAP search filter (RFC 4515).

## What is built

- `springboard/ui/src/directory_contacts.rs`: `Contact` (read from a person record), `escape_filter_value`,
  `contact_filter` (the filter text) and `search_contacts` (the query over records the caller holds).
- Records and nodes come from `directory_query.rs` and `directory_node.rs`.
- Headless tests: `springboard/ui/tests/parity_sdk-complete-03.rs`.

## What needs the lab machine

- A directory server client that sends `contact_filter` output and maps the returned entries into `DirRecord`s.
- Wiring the contacts app to search a directory node as well as its local contacts.

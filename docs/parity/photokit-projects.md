# Photo projects (PhotoKit project model)

Mirrors the project part of PhotoKit: a project is a titled list of sections, and each section is an ordered list of
asset identifiers. Changes are applied as lists, and a list that fails part way leaves the project unchanged.

## What is wired

- `springboard/ui/src/photo_project.rs`: `Project`, `Section`, `Change`, `Error`. Names are trimmed and bounded
  (`MAX_TEXT`), a project holds at most `MAX_SECTIONS` sections, and an asset may sit in more than one section.
- Tests: `springboard/ui/tests/parity_sdk-rest-03.rs`, headless, no photo library involved.

## What needs more work

- No extension host (`PHProjectExtensionController`): nothing lets a third-party extension create or edit a project.
- The photo library in the shell does not store projects, so a project lives only in memory.
- Rendering a project as a photo book or calendar is not part of this model.

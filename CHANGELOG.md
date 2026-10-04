# Changelog

Notable changes to done. The project is pre-1.0: minor versions (0.1 to 0.2) may
break the API, and every breaking change is listed under **Breaking** with a
migration note.

## [0.1.0] - unreleased

The first public version. **Unfinished:** Linux/GTK 4 works and is tested; the
Windows and macOS backends compile but have never been run.

### Added

- Declarative views as typed Rust values, reconciled into retained native
  controls, with property-by-property diffing and one rebuild per event batch.
- Widgets: `text`, `button`, `text_input`, `checkbox`, `slider`, `progress`,
  `dropdown`, `image`, `scroll` (vertical).
- Layout on taffy's flexbox: `row`, `column`, `stack`, gap, padding, grow and
  alignment (start, center, end, stretch), measured by the native controls.
- `keyed(..)` collections that keep native widgets and keyboard focus across
  insertions, removals and reorders; `Option` and `Either` views.
- Controlled inputs: accepted edits send no setter back (caret and IME state
  survive), rejected edits revert, and programmatic changes never echo as events.
- GTK 4 backend for Linux (GTK 4.10 or newer).
- Win32 and AppKit backends: compile and lint only, never run.
- Examples: `hello`, `counter`, `todo`, `settings`.
- Mock-backend core tests, native GTK tests, and a size/memory/startup
  measurement script with milestone reports.
- Released into the public domain under the Unlicense.

### Known limitations

- Windows and macOS have never been run.
- No background-work API, no text wrapping, vertical scrolling only, no menus,
  dialogs or multiple windows.
- Accessible names are not verified through a screen-reader API. Win32
  label-only accessible names and dark mode are missing, and macOS has no
  application menu.
- Tested on one Linux machine (X11, GTK 4.22.5, Rust 1.98.1); Wayland and the
  declared minimum versions are untested.
- Cold-start time is unmeasured. GTK 4 alone exceeds the project's 30 MB
  idle-memory target.
- Not published on crates.io: the name `done` is taken there.

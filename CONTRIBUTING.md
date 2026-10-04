# Contributing to done

Thanks for looking. done is a small, early project with one maintainer, so the
process is light: open an issue, talk it through if it is big, send a focused
pull request. Not every contribution will be merged. The project keeps a narrow
scope on purpose (see "What fits" below), but every report and patch will be
read.

The most useful contributions right now are **running the Windows and macOS
backends**, which have never run on real machines. If you are willing to test
them, feel free to make pull requests with fixes, or open an issue reporting
exactly what happened.

## Prerequisites

- A stable Rust toolchain. The manifests declare Rust 1.92; development uses the
  current stable release (1.98 at the time of writing).
- On Linux: GTK 4 development files, version 4.10 or newer, and `pkg-config`
  (Debian/Ubuntu `libgtk-4-dev`, Arch `gtk4`, Fedora `gtk4-devel`).
- On Windows and macOS, nothing beyond the platform itself: `windows-sys` and
  `objc2` are plain bindings.
- For the native tests: a display **and a window manager** (any desktop session
  works, or the Xvfb recipe below).

## Build, test, run

```sh
git clone https://github.com/don2e4/done-rust
cd done-rust
cargo build --workspace --all-targets
cargo test --workspace
cargo run -p done --example hello
```

`cargo test --workspace` runs the core tests (`crates/done-core/tests/reconcile.rs`).
They drive the framework against a recording mock backend, need no GTK and no
display, and are where most behaviour is pinned down.

The native GTK tests are marked `#[ignore]` because they open real windows. Run
them explicitly, one at a time:

```sh
cargo test -p done --tests -- --ignored --test-threads=1
```

Without a desktop, use a virtual display with a window manager (some tests resize
the window, which needs one):

```sh
Xvfb :99 -screen 0 1280x720x24 &
DISPLAY=:99 openbox &
DISPLAY=:99 cargo test -p done --tests -- --ignored --test-threads=1
```

Each native test file runs in its own process, because GTK binds to the thread
that first initializes it. Add new native tests as new files under
`crates/done/tests/`.

The examples are `hello`, `counter`, `todo` and `settings`:

```sh
cargo run -p done --example todo
```

### Checking Windows and macOS from Linux

Both backends must keep type-checking even if you only have Linux:

```sh
rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --target aarch64-apple-darwin
```

This only proves the code compiles. On a real Windows or Mac, `cargo build
--workspace --all-targets` and running the examples is much more valuable. Pull
requests that make those backends actually work are very welcome; if you can only
report results, good or bad, please open an issue.

## Repository layout

| Path | What lives there |
| --- | --- |
| `crates/done` | The public crate: prelude, `app()`, compile-time backend selection, examples, native tests |
| `crates/done-core` | Views, reconciliation, events, layout. Never depends on a toolkit |
| `crates/done-gtk4`, `done-win32`, `done-appkit` | One backend each, behind the internal `Backend` trait |
| `ARCHITECTURE.md` | The design and the backend contract |
| `measurements/` | The size/memory/startup script and the milestone reports |
| `demo/` | The demo video and the script that recorded it |

## Style

- `cargo fmt --all` and `cargo clippy --workspace --all-targets` with no warnings.
- `unsafe` is forbidden (`#![forbid(unsafe_code)]`) everywhere except the Win32
  and AppKit backends. There, every `unsafe` block needs a `// SAFETY:` comment
  explaining why it is sound; clippy enforces this with
  `deny(clippy::undocumented_unsafe_blocks)`.
- Match the surrounding code: its naming, its comment density, its idioms.
  Comments explain *why*, not *what*.
- No new dependencies without discussion. The runtime dependencies are the
  platform bindings and taffy, on purpose; [ARCHITECTURE.md](ARCHITECTURE.md)
  section 2 explains the policy.
- Behaviour changes in core come with a mock-backend test in
  `crates/done-core/tests/reconcile.rs`. Native behaviour (focus, IME, layout
  against real widgets) comes with a native test. A useful habit: break the code
  you are protecting and check that your test fails.

## Reporting bugs

Open an issue with the bug template: the commit you used, your OS, display server
and GTK version, the smallest program that shows the problem, what you expected
and what happened. "It doesn't build on Windows" with the full compiler output is
a welcome bug report.

## Proposing features

Open an issue that starts from the **problem**: what you are trying to build and
where done gets in the way. A concrete use case is worth more than a proposed API.

## What fits

done is deliberately small: native widgets only, no webview, no custom drawing
engine, no styling language, few dependencies. Proposals that move away from
those points will probably be declined, however good the code. If you are unsure,
ask first.

## Pull requests

- Keep each PR to one change. Separate refactors from behaviour changes, and keep
  unrelated formatting out of the diff.
- Write commit messages that say what changed and why.
- **Discuss larger changes first:** anything that changes the backend contract
  (`Backend`, `Properties`, `Change`, `Event`), the public API, or the layout
  model. An issue before the code saves everyone time.
- Before submitting, run `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets` and `cargo test --workspace`, plus the
  native tests if you touched the GTK backend or anything they cover.

## Breaking changes

Until 1.0, the public API can change between minor versions (0.1 to 0.2).
Breaking changes are allowed, but they must be deliberate. Explain the reason in
the PR and add an entry under a **Breaking** heading in
[CHANGELOG.md](CHANGELOG.md) with a short migration note.

## Keeping documentation true

When behaviour changes, update the documents that describe it in the same PR:

- `README.md` for anything user-visible: features, limitations, examples.
- `ARCHITECTURE.md` for the backend contract, layout semantics, or a design
  decision.
- `CHANGELOG.md` for every user-visible change.

The README's "What doesn't (yet)" section matters as much as its feature list.
If you fix a limitation, remove it; if you find one, add it.

## License

done is released into the public domain under the [Unlicense](LICENSE). By
contributing, you agree that your contribution is released under the same terms.

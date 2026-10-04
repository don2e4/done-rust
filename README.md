# done

**Declarative desktop GUIs in Rust, drawn entirely by the operating system's own widgets.**

You describe a window as a plain Rust value, such as `column((text(..), button(..)))`,
and done keeps a tree of real native controls in sync with it. On Linux those are
`GtkButton` and `GtkEntry`; on Windows, `BUTTON` and `EDIT` child windows; on macOS,
`NSButton` and `NSTextField`. There is no browser, webview, JavaScript runtime, HTML
or application CSS. The toolkit draws every pixel and handles text input, IME and
accessibility; done only owns your state, the reconciliation and the layout.

> **Status: unfinished.** This is an early v0.1. Linux/GTK4 works and is tested. The
> Windows and macOS backends compile but **have never been run**. If you are willing
> to test them, feel free to open pull requests. The API will change. Not published
> on crates.io.

**What it's for:** small desktop tools that should look and behave like native
apps, with the system's keyboard handling, screen-reader roles and text editing,
without shipping a browser engine. A hello-world release binary is about 450 KB
on Linux, with GTK itself linked as a shared system library.

**Who might enjoy it today:** Rust developers on Linux who want to try a tiny
retained-mode UI layer; anyone curious how a React-style "describe it, diff it"
model maps onto real native widgets; and people with Windows or Mac machines who
would like to be the first to run those backends.

A 29-second [demo video](demo/done-demo.mp4) goes from an empty directory to a
running native window (MP4; GitHub offers it as a download).

## A whole app

```rust
use done::prelude::*;

fn main() -> done::Result<()> {
    done::app("Counter", 0, |count: &i32| {
        column((
            text(format!("Clicked {count} times")).size(24.0),
            button("+1").on_click(|count: &mut i32| *count += 1),
        ))
        .gap(8.0)
        .padding(16.0)
        .align(Align::Center)
    })
    .run()
}
```

The closure describes the window for a given state. A click handler gets
`&mut State`; after a batch of events done calls the closure once, compares the
new description with the last one, and patches only what changed. Here that is
one label's text, with the same native label kept.

## Try it

You need a stable Rust toolchain, the GTK 4 development files (4.10 or newer) and
`pkg-config`. On Debian and Ubuntu that is `libgtk-4-dev`; on Arch, `gtk4`; on
Fedora, `gtk4-devel`.

```sh
git clone https://github.com/don2e4/done-rust
cd done-rust
cargo run -p done --example hello
```

More examples in [crates/done/examples](crates/done/examples):

| Example | Shows |
| --- | --- |
| `hello` | the smallest app |
| `counter` | state, buttons, row and column layout |
| `todo` | a keyed list in a scroll view; rows keep their native controls and keyboard focus when items move |
| `settings` | a form of controlled inputs: text input, checkbox, slider, drop-down, progress bar, image, stack |

Run any of them with `cargo run -p done --example <name>`.

## Use it in your own project

The crate name `done` on crates.io belongs to an unrelated project, so depend on
this repository directly:

```toml
[dependencies]
done = { git = "https://github.com/don2e4/done-rust" }
# or, with a local checkout:
# done = { path = "../done-rust/crates/done" }
```

Two things to know:

- **Components and Rust 2024.** In edition 2024 (the default for `cargo init`), a
  function `fn view(state: &State) -> impl View<State>` captures the borrow of
  `state` in its return type and is rejected by `done::app`. Take the state by
  value, or write `-> impl View<State> + use<>`:

  ```rust
  use done::prelude::*;

  fn counter(n: i32) -> impl View<i32> {
      column((
          text(format!("Clicked {n} times")),
          button("+1").on_click(|n: &mut i32| *n += 1),
      ))
  }

  fn main() -> done::Result<()> {
      done::app("Counter", 0, |n: &i32| counter(*n)).run()
  }
  ```

- **Release builds.** The small binaries above come from the release profile in
  this workspace's [Cargo.toml](Cargo.toml) (`opt-level = "z"`, LTO, one codegen
  unit, `panic = "abort"`, strip). A dependency cannot set its consumer's profile,
  so copy it into your own `Cargo.toml` if you want the same result. On Windows
  (MSVC), also embed [windows.manifest](crates/done/windows.manifest) from your
  build script for common-controls v6 and per-monitor DPI awareness:

  ```rust
  // build.rs, next to a copy of windows.manifest
  fn main() {
      let target = std::env::var("TARGET").unwrap_or_default();
      if target.ends_with("-windows-msvc") {
          let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
          println!("cargo::rustc-link-arg-bins=/MANIFEST:EMBED");
          println!("cargo::rustc-link-arg-bins=/MANIFESTINPUT:{dir}/windows.manifest");
      }
  }
  ```

## How it works

The interesting part is the split. **Core owns state, identity and geometry; the
toolkit owns pixels, input and accessibility.**

- **Views are short-lived typed values.** `column((text(..), button(..)))` is a
  nested tuple type, not a boxed node tree. Rebuilding produces a fresh value that
  is diffed property by property against a small retained record (native widget
  ID, last applied values, children) and then dropped. Closures are refreshed on
  every rebuild without reconnecting native signals.
- **Core does the layout, natively measured.** done asks each native control for
  its minimum and natural size, runs flexbox ([taffy](https://github.com/DioxusLabs/taffy),
  with only its `std` and `flexbox` features), and hands every control an exact
  rectangle. Taffy runs over core's own retained tree through its low-level traits
  instead of a second copy in `TaffyTree`. That makes room for two small custom
  layouts flexbox cannot express: a `stack` that is as large as its largest
  overlapping child, and a scroll viewport.
- **Inputs are controlled, but never fight the user.** When you type, the native
  control already shows the new text. done records that as the control's current
  value before rebuilding, so if your state accepts the edit, no setter is sent
  back and the caret, selection and IME composition survive. If your state rejects
  or changes the value, the control snaps to what you described. Programmatic
  changes never come back as events (tested on GTK).
- **Events are batched.** Native callbacks only enqueue `(widget, event)`. One
  scheduled flush runs every handler in order, rebuilds once, reconciles once and
  lays out once.
- **Keyed lists keep identity.** `keyed(items.map(|item| (item.id, row(..))))`
  matches children by key: survivors are moved, not recreated, so a row that moves
  keeps its native widget and its keyboard focus (tested on GTK). Duplicate keys
  are an error.

The backend contract is about seven operations (create, update, destroy, set
frame, measure, post, run loop). Each backend is roughly 800 to 1,900 lines; core
is about 2,600. [ARCHITECTURE.md](ARCHITECTURE.md) is the design document and
contract.

## What works today

- **Widgets:** `text`, `button`, `text_input` (placeholder, accessible label, input
  and submit handlers), `checkbox`, `slider` (range, step), `progress`, `dropdown`,
  `image` (local PNG and similar; load failures are returned as errors), `scroll`
  (vertical).
- **Layout:** `row`, `column`, `stack`, `.gap()`, `.padding()`, `.align()` (start,
  center, end, stretch), `.grow()`. The window cannot be shrunk below the layout's
  native minimum size.
- **Structure:** tuples of up to 12 children, `Option<V>` for optional children,
  `Either<L, R>` to switch view types, `keyed(..)` for dynamic lists.
- **Behaviour:** batched events; Tab order follows source order; accessible roles
  come from the native controls; the layout follows window resizes and font, text
  scale and theme changes (GTK). `run()` returns an error instead of panicking when
  the toolkit or an image fails.

## What doesn't (yet)

- **Windows and macOS have never run.** Both backends type-check and pass clippy
  from the same example sources, and that is all. Their sizing, DPI handling,
  keyboard behaviour and image decoding are untested;
  [MILESTONE_3.md](measurements/MILESTONE_3.md) has the full list.
- **No background work API.** There is no way yet to run work off the UI thread and
  deliver the result; the design is in [ARCHITECTURE.md](ARCHITECTURE.md), but it
  is not built.
- **Small widget set.** Text does not wrap; scrolling is vertical only; there are
  no menus, dialogs, multi-window apps, custom drawing, styling API or
  virtualized lists.
- **Accessibility is the toolkit's,** and only partly checked: roles are tested on
  GTK, but accessible names were never read back through a screen-reader API. On
  Windows, label-only accessible names and dark mode are not implemented. On
  macOS there is no application menu (Command-Q does nothing).
- **Linux testing is narrow:** one machine, X11 with Openbox, GTK 4.22.5,
  Rust 1.98.1. Wayland, other distributions and the declared minimums (Rust 1.92,
  GTK 4.10) are untested.
- **Memory:** the project's own target of 30 MB idle memory is not met, because GTK 4
  itself idles at about 72 to 78 MB on the test machine. done adds roughly 0.1 MB on top.

## Platforms

| Platform | Status |
| --- | --- |
| Linux, GTK 4, X11 | Built, tested (unit and native tests) and run |
| Linux, Wayland | Not tested |
| Windows (`x86_64-pc-windows-msvc`) | Compiles and lints from Linux; never run |
| macOS (`aarch64-apple-darwin`) | Compiles and lints from Linux; never run |

## Measurements

[measurements/milestone1.py](measurements/milestone1.py) compares a done
hello-world with a hand-written GTK 4 hello-world built from the same bindings,
same text and same window size. It alternates the two in fresh processes, reads
idle memory as PSS from `/proc/<pid>/smaps_rollup`, and times startup up to GTK's
`after-paint` signal. On one Linux desktop (x86_64, X11, GTK's GL renderer):

| | done hello | hand-written GTK hello |
| --- | --- | --- |
| Release binary, stripped | 453,856 B | 315,960 B |
| Idle memory (PSS, median) | 78.2 MB | 78.0 MB |
| Startup to first `after-paint`, warm cache (median / p95) | 113 ms / 170–185 ms | 112 ms / 167 ms |

Binary sizes are from the current tree. Memory and startup come from the
[milestone 2 runs](measurements/MILESTONE_2.md#measurements), when the hello binary
was 64 bytes smaller. done's p95 is the range across two runs; the hand-written
hello's is from one. The memory difference is within run-to-run noise (paired
median +0.13 MB). Cold
start has not been measured, and the `after-paint` marker has not been checked
against actual presentation. There is no comparison with Electron or any web
framework. The [milestone reports](measurements/README.md) have the raw method,
every sample and the caveats.

## Roadmap

Near-term, in rough order:

1. Run the Windows and macOS backends on real machines, fix what breaks, and port
   the GTK native tests to them.
2. Measure cold start properly and check the first-frame marker against real
   rendering.
3. A small API for background work whose results come back on the UI thread,
   as designed in [ARCHITECTURE.md](ARCHITECTURE.md).
4. Repeatable benchmark automation on all three platforms.

## Repository layout

```text
crates/done         the public crate: prelude, app builder, backend selection, examples
crates/done-core    views, reconciliation, events, layout (no OS toolkit needed)
crates/done-gtk4    Linux backend
crates/done-win32   Windows backend (compiles; never run)
crates/done-appkit  macOS backend (compiles; never run)
measurements/       measurement script and milestone reports
demo/               the demo video and the script that recorded it
```

## Contributing

Bug reports and focused pull requests are welcome; see
[CONTRIBUTING.md](CONTRIBUTING.md). **Windows and macOS testers especially:** if you
are willing to build and run those backends, feel free to make pull requests with
fixes, or open an issue with what happened, even if it is only "it doesn't
compile" and the error. Please open an issue before larger changes, especially to
the backend contract.

## License

done is free and unencumbered software released into the public domain under the
[Unlicense](LICENSE).

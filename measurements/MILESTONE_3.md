# Milestone 3 report

Win32 and AppKit backends, with the counter, todo, settings and hello examples
building from identical Rust sources on all three platforms.

**The new backends compile; they have not run.** This machine is Linux with no
Windows or macOS system, no Wine, and no Windows or Apple SDK to link against.
Everything below that is marked verified was verified by type-checking and
linting, not by running. ARCHITECTURE section 9 already notes that milestone 3
needs real OS runners, and that cross-compilation alone cannot establish runtime
results.

As with milestone 2, the plan-first and stop points were not held, because you
asked for milestones 2 and 3 in one go.

## Plan as carried out

1. Before milestone 2 froze the backend contract, check every operation against
   Win32 and AppKit. This found two gaps, both now in the contract:
   - `Backend::scrollbar_gutter()`, default 0: Win32 scroll bars take space from
     the viewport, so content is laid out narrower by that width. GTK and macOS
     overlay scrollers report 0; macOS reports the legacy scroller width when the
     system uses legacy scrollers.
   - `Properties::Group { overlap }` and `Change::Overlap`: in Win32, one sibling
     order decides both Tab order and stacking, in opposite directions. A stack
     host reverses its stacking so later children are drawn on top.
2. `done-win32` (`windows-sys` 0.61) and `done-appkit` (`objc2` 0.6,
   `objc2-foundation` 0.3, `objc2-app-kit` 0.3).
   - Both have target-specific dependencies and `#![cfg]`-gated sources, so each
     compiles to nothing on other platforms.
   - Both enable explicit feature lists: no `windows-sys` default modules and no
     AppKit defaults.
3. Facade: features `gtk4`, `win32` and `appkit`, all enabled by the default
   `native` feature. The backend is chosen at compile time, and an unsupported
   target fails with a clear `compile_error!`.
4. Type-check and lint every example for `x86_64-pc-windows-msvc` and
   `aarch64-apple-darwin` with an official toolchain.

## What was verified

| Check | Linux (GTK4) | Windows (`x86_64-pc-windows-msvc`) | macOS (`aarch64-apple-darwin`) |
| --- | --- | --- | --- |
| hello, counter, todo, settings compile unchanged | Built and run | **Type-checked** | **Type-checked** |
| clippy, warnings | 0 | 0 | 0 |
| every `unsafe` block has a `SAFETY:` comment | n/a (`forbid(unsafe_code)`) | Enforced (`deny(clippy::undocumented_unsafe_blocks)`) | Enforced |
| core tests (15, mock backend) | Pass | Platform-independent | Platform-independent |

- Toolchain: official Rust 1.98.1, the same commit (48a229cea) as the system
  compiler. Installed with a checksum-verified `rustup-init`, with the Windows and
  macOS standard libraries and clippy, into `target/xtool/` (890 MB).
- Cross build artifacts go to `target/cross/` (125 MB). Both directories are
  gitignored and can be deleted; nothing outside the repository was changed.
- Command: `target/xtool/xcargo clippy --workspace --all-targets --target <triple>`.
  The whole workspace builds for each target, because every backend crate
  (GTK4 included) has target-specific dependencies and `#![cfg]`-gated sources.
  A real Windows or macOS machine can therefore run `cargo test` and `cargo
  clippy` at the root.

The two contract additions changed the Linux build only slightly: after
milestone 3 the release hello is 453,856 B (milestone 2: 453,792 B), and all
four GTK native test binaries still pass on the desktop.

## What is not verified

Every runtime behaviour on Windows and macOS. In particular:
- **Linking.** `cargo check` does not link. The Windows manifest embedding
  (common controls v6 and per-monitor v2, in `crates/done/build.rs` for this
  package's examples) has not produced a binary.
- **Rendering, layout and measurement.** The Win32 sizes are calculated: text
  extents with the message font, `BCM_GETIDEALSIZE` for buttons, system metrics
  for check boxes, and fixed logical sizes for trackbars, progress bars and edit
  fields. AppKit sizes come from `fittingSize`. None of these has been compared
  with real rendering.
- **DPI.** Win32 sets per-monitor v2 awareness, recreates fonts on `WM_DPICHANGED`
  and converts logical units with edge rounding. Never exercised.
- **Keyboard.** Win32 uses `IsDialogMessage` with `WS_EX_CONTROLPARENT` hosts and
  an edit subclass that turns Enter into Submit. AppKit builds a source-order
  key-view loop after every batch. Neither has been exercised.
- **Events.**
  - Win32: `WM_COMMAND`, `WM_HSCROLL` and a posted `WM_FLUSH`, with `EN_CHANGE`
    suppressed during programmatic text changes.
  - AppKit: target-action, the text-field and window delegates, and
    `performSelector:...inModes:` in the common modes.
- **Images.** Win32 decodes through WIC with hand-written COM vtable slots,
  because `windows-sys` declares no COM interfaces. The CLSID, IID and pixel
  format GUID were transcribed from `wincodec.h` and are unchecked.
- **Measurements.** No size, memory or startup numbers exist for Windows or macOS.

## Known gaps by design

- **Win32 accessible names.** Windows names an input after the label that
  precedes it. `.label()` names without visible text are not mapped yet; that
  needs `IAccPropServices`, which means more hand-written COM.
- **Win32 dark mode.** Standard USER32 and common controls have no documented
  dark-mode API, and ARCHITECTURE forbids undocumented ones and owner-drawn
  replicas. Controls follow the classic or visual-styles light appearance.
- **Win32 stacks.** Overlapping children are stacked later-on-top, so Tab order
  inside a stack runs last to first.
- **Win32 images.** Static bitmap controls ignore alpha, so images are blended
  onto the button-face colour when loaded.
- **AppKit.**
  - Push buttons take keyboard focus only when the user enables Full Keyboard
    Access; that is system behaviour.
  - There is no application menu, so Command-Q does nothing; close the window.

## Next steps that need real machines

1. Run the examples on Windows 10 and 11, and on macOS 13 or later (Intel and
   Apple Silicon). Check the items in "What is not verified" one by one.
2. Port the GTK native tests to each backend: batching, identity, focus, no
   echo, keyed reorder, scrolling, images.
3. Rerun the hello-world gates on each OS with that OS's memory metric:
   Windows private working set and macOS `footprint`, as ARCHITECTURE section 7
   requires.

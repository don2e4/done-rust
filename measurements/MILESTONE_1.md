# Milestone 1 report

Linux / GTK4 window, text and button, with the counter example and a minimal
measured row/column layout. This report stops at the milestone 1 stop point.
Nothing from milestone 2 was started.

## Summary

| Area | Status |
| --- | --- |
| Core reconciliation tests (mock backend, no display) | **Pass**, 5 of 5 |
| Native tests on a real desktop (2 tests) | **Pass**, repeated runs; see "What the native tests prove" |
| Counter example, screenshot and real pointer clicks | **Works** |
| Release binary size | **Pass**: 370,424 B against a 2,000,000 B limit; also under the 800 KB stretch goal |
| Idle memory, absolute 30 MB limit | **Fails for the toolkit itself**: raw GTK4 idles at about 72 MB. Passes only through the toolkit-baseline exception, which needs your decision |
| Framework overhead against raw GTK4 | **Pass**: indistinguishable from zero (+0.01 MB and -1.85 MB run medians; limit 5 MB) |
| Cold launch to first frame, 150 ms | **Unverified.** Warm-cache p95 is already 169-185 ms for the default renderer (raw GTK4: 147-169 ms), so a pass is unlikely. There is no exception for startup |
| Layout host at changing size and font | **Partly validated**; scale factors, themes and AT-SPI child order are not |

ARCHITECTURE section 7 says a hard-budget failure stops the milestone, with the
measured values shown and options proposed. Both are below. I have not proceeded
past this point.

## Environment

| | |
| --- | --- |
| CPU / RAM | 12th Gen Intel Core i3-12100 (8 threads), 16,515,133,440 B |
| OS | Linux 7.2.7-273-tkg-bore, glibc 2.44 (Arch) |
| Toolchain | rustc 1.98.1, cargo 1.98.1 |
| Toolkit | GTK 4.22.5 |
| Session | Real Xorg on `:0` (lightdm), openbox window manager, picom compositor. **Not** a virtual display |
| Display / scale / renderer | `GdkX11Display`, scale 1, `GskGLRenderer` (the session default; nothing overridden) |
| Conditions | A live desktop, not a dedicated measurement host. Warm OS caches |
| Build identity | The workspace is not a Git repository. Source and binary SHA-256 values are in `measurements/local/linux.json` |

The raw JSON, screenshots, and each run are in `measurements/local/`, which is
gitignored. The tables below are generated from those files.

## Commands

```sh
cargo test -p done-core --test reconcile
cargo test -p done --tests -- --ignored --test-threads=1      # native, needs a desktop
cargo build --release -p done --examples -p done-gtk4 --example raw_hello
cargo build --profile probe -p done --example hello -p done-gtk4 --example raw_hello --features done/probe,done-gtk4/probe
python3 measurements/milestone1.py                                   # runs 1 and 2
python3 measurements/milestone1.py --renderer cairo --output measurements/local/linux-cairo-informational.json
```

The script opens about 40 short-lived windows per run. Run 1 was made by the
code-review agent on a tree identical to the one measured in run 2 (source hashes
match). Only test files changed afterwards, and the measured binaries are
byte-identical to the current build.

## Binary size

| Binary | Bytes |
| --- | ---: |
| `done` hello, release (the gate metric) | 370,424 |
| `done` counter, release | 375,776 |
| raw GTK4 hello, release | 315,960 |
| `done` hello, probe build | 376,120 |
| raw GTK4 hello, probe build | 322,152 |

- Hard limit 2,000,000 B; stretch goal 800,000 B. Both are met.
- Framework cost over the matching raw GTK4 hello, release: 54,464 B.
- Instrumentation effect, reported separately from the shipping build: the probe
  build is 5,696 B larger than the shipping build.
- The executable is stripped and dynamically linked. It needs `libgtk-4.so.1`,
  `libpango-1.0.so.0`, `libgraphene-1.0.so.0`, `libgio-2.0.so.0`,
  `libgobject-2.0.so.0`, `libglib-2.0.so.0`, `libgcc_s.so.1` and `libc.so.6`. A
  recipient needs a compatible GTK4 installation; the executable size alone is not
  a self-contained distribution size.

## Idle memory (PSS from `smaps_rollup`)

Five alternating pairs per run, five samples one second apart after a two-second
wait. Medians are per probe; overhead is the median of the paired differences.

| Run | done idle PSS median (B) | raw GTK idle PSS median (B) | paired overhead median (B) | paired overheads (MB) | max sample done / raw (MB) |
|---|---:|---:|---:|---|---|
| 1 (review agent) | 70,118,400 | 71,967,744 | -1,846,272 | +0.28, -2.12, -1.85, +0.10, -1.89 | 72.09 / 72.11 |
| 2 | 70,320,128 | 72,087,552 | 14,336 | +0.01, +0.15, +2.09, -1.93, -1.96 | 72.32 / 72.25 |
| cairo (informational, separate configuration) | 57,901,056 | 57,795,584 | 120,832 | +0.18, +0.12, -0.42, +0.40, +0.02 | 58.25 / 58.30 |

- Limits: 30 MB absolute (stretch 15 MB), 5 MB over the matched native hello
  (stretch 2 MB).
- **Absolute limit:** `done` is at about 70 MB and so is the raw GTK4 hello. The
  toolkit baseline alone makes the absolute limit impossible with the default
  renderer on this host. ARCHITECTURE section 7 lets the overhead gate take
  precedence in that case, so the script reports a pass. Both absolute numbers are
  shown here as that section requires.
- **Overhead:** the paired differences range from about -2 MB to +2 MB, so with
  five pairs the framework's cost is not distinguishable from zero. It is well
  inside the 5 MB limit. I would not claim the 2 MB stretch goal from this sample.
- **Renderer:** the cairo run is a separate configuration, never mixed with the
  gate numbers. It lowers both probes by about 12 MB, and raw GTK4 still idles at
  about 58 MB. So the 30 MB limit is not reachable with GTK4 on this host under
  either renderer. This answers the renderer question in ARCHITECTURE section 9
  for this host only.

## Startup (fresh process, warm OS cache)

Twenty alternating pairs per run, measured from a monotonic timestamp taken before
process creation to GTK's `after-paint` marker.

| Run | process | median (ms) | p95 (ms) | max (ms) | n |
|---|---|---:|---:|---:|---:|
| 1 (review agent) | done | 120.7 | 169.1 | 183.4 | 20 |
| 1 (review agent) | raw | 114.8 | 147.0 | 170.5 | 20 |
| 2 | done | 122.4 | 185.4 | 186.8 | 20 |
| 2 | raw | 123.4 | 168.7 | 180.2 | 20 |
| cairo (informational) | done | 76.8 | 117.9 | 129.3 | 20 |
| cairo (informational) | raw | 78.9 | 115.6 | 119.7 | 20 |

- **The cold-start gate is unverified.** No cache reset was done: that needs a
  dedicated host and was deliberately not attempted on a live desktop. The gate
  uses cold-run p95, and these are warm.
- **The frame marker is unvalidated.** `after-paint` has not been checked against
  actual rendering. Compositor presentation is not measured.
- Even so, warm p95 for the default renderer is above 150 ms for `done` in both
  runs, and for raw GTK4 in run 2. Cold runs are not faster than warm runs, so a
  cold pass is unlikely. `done` and raw GTK4 are within run-to-run noise of each
  other, which points at toolkit and renderer startup rather than the framework.
- The run-to-run spread is large (`done` p95 169 ms against 185 ms), so no single
  p95 above should be quoted as the figure.

## Tests

### Core, headless

`cargo test -p done-core --test reconcile` runs five tests against a recording
mock backend. They cover the initial hierarchy, one rebuild for two queued clicks,
a text-only update patching one property, an identical view emitting no native
operations, handler refresh without recreating controls, resize repositioning
without remeasuring, theme change remeasuring, font/label/gap patching only the
affected widgets, and destruction order on shutdown and on a patch failure.

### What the native tests prove

`cargo test -p done --tests -- --ignored --test-threads=1` runs two tests, each in
its own process. GTK binds to the thread that initialises it and the harness gives
each test its own thread, so two GTK tests cannot share a binary. They need a
desktop and a window manager. After the fixes listed below, both passed in five
consecutive runs of the combined command; before them the first test aborted.

`native_counter_batches_reuses_widgets_and_preserves_focus`:

- Two native clicks produce one rebuild and the label reads "Clicked 2 times".
- The label and both buttons are the same GTK objects before and after the patch.
- The programmatic Tab move from the first to the second button follows source
  order, and the second button is still the window's focus widget after the patch.
- Accessible roles are Label and Button.
- Each control's border box is at least its measured minimum width and has a
  non-zero height.

`native_layout_follows_window_size_and_font_changes`:

- Resizing the window from 320 to 500 wide re-centres the label.
- Changing the GTK font name makes the controls grow, and the layout re-allocates
  to the new minimum and stays centred.
- I checked that this test can fail. With core ignoring `AppearanceChanged` it
  fails in the font-change step, and with core ignoring `Resized` it fails in the
  resize step, each with a clear message. The framework was restored byte for byte.

What they do not prove:

- Focus is checked with `is_focus`, so it means the window's focus widget, not
  that the window manager has activated the window. Tab is the programmatic
  `child_focus`, not real key events.
- Clicks in the tests are `emit_clicked`. Separately, I ran the release counter,
  captured its window, and sent three real pointer clicks to "+"; the label read
  "Clicked 3 times" and the process exited cleanly.
- Accessible child order was not inspected through AT-SPI.
- Scale factors other than 1, other themes, and right-to-left direction were not
  tested. IME and selection do not apply yet, because there is no text input.

### Static checks

`cargo fmt --check` is clean and `cargo clippy --workspace --all-targets` is clean
both with and without `--features done/probe,done-gtk4/probe`. Searching the native
test output for GTK warnings and criticals found none.

## Dependencies

- One direct external runtime dependency: `gtk4` 0.11.5 with default features off,
  in `done-gtk4`. The facade's `gtk4` entry is a dev-dependency used by the native
  tests only.
- The normal and build graph has 62 crates, 59 of them external. They are the
  `gtk4`, `gdk4`, `gsk4`, `glib`, `gio`, `pango`, `cairo`, `graphene` and
  `gdk-pixbuf` binding crates and their `-sys` crates, `futures-*`, and the
  build-time and proc-macro crates (`syn`, `quote`, `proc-macro2`, `system-deps`,
  `toml*`, `cfg-expr`, `pkg-config` and similar).
- Nothing outside the allowance was added. I did not measure the per-crate size or
  memory delta, because the gtk4 bindings are the only dependency and the raw
  probe uses the same bindings.

## Defects found and fixed in this milestone

- `crates/done/tests/native.rs` aborted the whole process on its first real run.
  Three test defects, none in the framework: `has_focus()` depends on window
  manager activation; `width()` is the CSS content box while `measure()` includes
  padding; and a 50 ms timer could read the label before the idle-priority batch
  ran. The tests now use `is_focus()`, the border box, and a bounded wait.
- `crates/done-gtk4/src/probe.rs` did not compile with the `probe` feature, because
  `window.display()` was ambiguous. No probe binary could be built, so
  `milestone1.py` had never run end to end. Fixed.
- `measurements/milestone1.py` counted `SwapPss:` lines in its `smaps` fallback and
  read only the main thread's children list when walking the process tree. Both
  fixed, with self-checks.
- README and ARCHITECTURE statements that no longer matched the repository were
  corrected.

## Known issues carried forward (not fixed)

These come from the code review. I verified the first two against the source and
my own measurements, and the rest are as the reviewer reported them.

- **Window minimum is 0.** `Host::measure` returns zero, so the toplevel's minimum
  size is 0 (observed). A user can shrink the window below its content. Core's
  computed minimum is never passed to the window, which conflicts with the
  native-minimum rule in ARCHITECTURE section 6. A clipped window was not
  demonstrated.
- **Tuples stop at four children.** `column((a, b, c, d, e))` fails with an opaque
  trait-bound error. A design limit until the keyed collections of milestone 2.
- **Resize lags one frame (reported).** `size_allocate` queues `Resized`, which is
  handled at idle priority, below GDK's redraw priority. The same ordering caused
  the timing flake described above, so the mechanism is real. Visible jitter during
  an interactive resize was not demonstrated.
- **Pre-realize measurements are cached (reported).** The review saw a label measure
  173 px before realization and 172 px after. If that ever went the other way the
  text would clip. The root fix is one `AppearanceChanged` on the window's first
  realize, but it adds a layout pass to the startup path that the gates measure.
  Left for you to decide.
- **Redundant snapshot override (reported).** `Host::snapshot` repeats GTK's
  default child snapshot behind an extra `abort_on_panic`. Simplification only.

## ARCHITECTURE section 9, status after milestone 1

| Question | Status |
| --- | --- |
| Can the allocation host apply exact core frames without conflicts, resize loops, or focus and accessibility regressions? | **Partly.** Exact frames, resize, font change, Tab order and roles verified; no resize loop seen. Scale, themes, AT-SPI order, and a clipped minimum-size case remain |
| Does the default renderer make absolute memory or startup unattainable? | **Memory: yes on this host**, under both renderers tried. **Startup: unverified** |
| Which first-frame signal is reliable? | **Open.** `after-paint` is not yet validated against rendering |
| Which minimum Rust and toolkit versions can be supported? | **Open.** Only Rust 1.98.1 and GTK 4.22.5 were tested |
| UI posting and spawn handle API | Not started; no worker example yet |
| Remaining questions | Belong to milestones 2 and 3 |

## Options and decisions needed

The absolute 30 MB limit and the 150 ms startup limit are the numbers the plan
asked me to flag. These are the choices, with my recommendation first.

1. **Accept the toolkit-baseline exception for memory and go on with the default
   renderer.** This follows ARCHITECTURE section 7 as written. `done` adds about
   zero over raw GTK4. The cost is that the absolute figure stays about 70 MB.
2. **Measure cold start properly before judging startup.** This needs a dedicated
   host and a deliberate cache reset, plus a check that `after-paint` matches real
   rendering. Do this before anyone claims or concedes the 150 ms gate. Warm
   numbers already suggest it will fail with the default renderer.
3. **Choose the renderer on purpose.** Cairo saves about 12 MB and about 45 ms of
   median warm startup in this sample, and still sits at about 58 MB. The plan forbids
   silent renderer substitution, so this needs your explicit approval, and it
   changes how GTK draws.
4. **Restate the Linux memory budget** relative to the matched native baseline,
   since the toolkit alone is about 2.4 times the stated absolute limit. This
   changes the contract and is yours to decide.

I recommend 1 together with 2. Ask for 3 or 4 only if the cold-start result also
fails.

Milestone 2 starts only after your go-ahead and its own plan.

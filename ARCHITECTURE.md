# done architecture

Status: milestone 0 design, amended where milestones 1 to 3 settled a question
(reports in `measurements/`). First backend: Linux / GTK4. Language: stable Rust.
The project and public crate are named `done` throughout.

## 1. Contract and boundaries

done describes an interface in Rust, reconciles it with retained native widgets,
and owns their layout. Win32, AppKit, and GTK4 provide drawing, accessibility,
IME, editing, and native interaction. There is no browser, system webview,
JavaScript runtime, HTML, or application CSS. GTK's internal theme machinery
remains part of GTK; done does not expose a CSS styling system.

The v0.1 widget set is window, text, button, single-line text input, checkbox,
slider, progress bar, dropdown, scroll, and image. Layout provides row, column,
stack, gap, padding, grow, and alignment. A stack overlays its children in source
order; it does not introduce a custom drawing surface. Ordinary keyed collections
support the todo example; virtualization belongs to v0.2.

The public API follows the supplied counter: `view(&State) -> impl View<State>`,
tuple children, builder methods, handlers receiving `&mut State`, and
`done::app(title, initial_state, view).run()`. `run` should return a standard
`Result` for initialization and native operation failures. There is no derive
macro, application markup parser, custom text engine, or async runtime.

## 2. Crates and dependency policy

Proposed workspace members:

| Crate | Responsibility | Dependencies |
| --- | --- | --- |
| `done` | Public prelude, app builder, target backend selection | Core and selected backend |
| `done-core` | Typed views, retained state, reconciliation, events, layout contract | Standard library; taffy (flexbox only) |
| `done-gtk4` | GTK widgets, native sizing, allocations, main loop | Core and `gtk4` |
| `done-win32` | HWND controls, message loop, DPI, WIC | Core and `windows-sys` |
| `done-appkit` | NSWindow/NSView controls, main loop, NSImage | Core and `objc2` / `objc2-app-kit`; necessary Foundation bindings |

Backends depend on core; core never depends on a backend or the public facade.
The facade selects a concrete backend at compile time, avoiding trait objects
for widget operations. Backends have no dependency on application state.

Facade features `gtk4`, `win32`, and `appkit` enable optional dependencies in
their respective target dependency sections. A default `native` feature enables
those target-gated selections, so examples build unchanged on each supported OS.
Only the host target's backend is compiled. Building core alone requires no OS
toolkit. Unsupported targets or a facade build without a backend produce a clear
compile-time error, rather than selecting another rendering technology.

Use only the approved platform bindings and taffy as direct external runtime
dependencies. Use bindings' reexports where available. Audit their necessary
transitive dependencies and enabled features; record them with measurements.
Choose versions and an MSRV when milestone 1 resolves and tests the actual
dependency graph, rather than claiming an untested compatibility range now.

Enable only taffy's `std` and `flexbox` features. Do not enable grid, block,
float, calc, markup/style parsing, serialization, debugging, or its `TaffyTree`
storage. [Taffy architecture and features](https://docs.rs/taffy/latest/taffy/)

Amended in milestone 2: core implements taffy's low-level tree traits over its
own retained nodes instead of mirroring them into `TaffyTree`. A stack sizes to
its largest overlapping child, which flexbox cannot express and which grid, being
excluded, cannot provide. Rows and columns use taffy's flexbox algorithm; stack
and scroll viewport are two small algorithms in core. This also drops the
`slotmap` dependency and the duplicate node storage. GTK bindings enable `v4_10`
for the GtkAccessible label API, so the minimum GTK version is 4.10.

Any dependency outside that allowance needs approval before addition, supported
by measured release-size delta in KB and idle-memory delta in MB. Build helpers
are dependencies too: prefer standard-library scripts and installed platform
tools, including Windows resource tools, before adding helper crates.

Applications, including examples, must use this workspace-root release profile:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Cargo profile settings in a library do not impose settings on consuming
applications. Document this profile for users and record any configuration or
environment overrides when benchmarking. [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)

## 3. Typed views and retained state

A description is a short-lived Rust value. Fixed children live in tuples;
leaf properties and handler closures live directly in their view structs. There
is no boxed universal node for every description and no independent heap-heavy
virtual DOM. Dynamic collections need a vector; formatted text needs owned
storage. These costs remain visible rather than hidden behind an allocation-free
claim. In v0.1, dynamic property strings are owned and literal strings may be
borrowed with `Cow<'static, str>`; handlers cannot borrow the temporary `&State`.

The core trait has the following shape. This is an interface sketch, not
compilable code or a frozen public signature; context types are omitted.

```rust,ignore
trait View<State> {
    type Retained;

    fn build<B: Backend>(self, cx: &mut BuildContext<State, B>)
        -> Result<Self::Retained, Error>;
    fn rebuild<B: Backend>(self, retained: &mut Self::Retained,
        cx: &mut BuildContext<State, B>) -> Result<(), Error>;
}
```

The view factory has one concrete return type. Tuple shape is static; optional
children and homogeneous keyed collections provide the variability needed by
the requested examples. Arbitrary runtime widget types and user-defined widget
extension APIs are not v0.1 requirements.

Retained state stores the last applied properties, native widget identity,
layout node identity, children, and event registrations. Each new view compares
against this state, patches it, and is consumed. Keeping the previous description
as a second complete tree is unnecessary. Closures are replaced independently
of property equality because closures do not have meaningful equality.

The milestone 1 implementation retains core IDs rather than backend handles, so
the associated retained type does not need a backend type parameter. Renderer
teardown walks the retained identity hierarchy; every view does not need to
duplicate that traversal. These keep the original typed design with fewer
generic parameters and teardown methods.

### Identity and patching

- Fixed children are matched by structural position and widget kind. Property
  changes preserve the native control. A different kind replaces that subtree.
- Dynamic children use caller-provided stable keys, unique among siblings.
  Match by key and kind, preserve surviving native controls, create additions,
  destroy removals, and update native child order only when it changes. Moving a
  key to another parent creates a new identity. Duplicate keys are an error.
- Use standard-library maps for keyed matching; avoid an O(n squared) search.
  Runtime `WidgetId` values are monotonically assigned and never reused during an
  app lifetime. A stale queued event cannot target a newly created control.
- Diff properties individually. A text-only counter update changes only the
  label text and any frames made necessary by its new intrinsic size. An
  identical view emits no native property or frame mutations.
- Structural deletion unregisters handlers before disconnecting callbacks and
  destroying native children, then releases layout state. Teardown is explicit
  and ordered; shared native wrappers must not create ownership cycles.

Native observations update the retained property's shadow before reconciliation.
For example, typing updates the known entry value; if state accepts that value,
rebuilding must not call a native text setter again. This preserves selection,
caret, and IME behavior. Programmatic setters suppress or identify their own
notifications so they do not trigger a second application update.

Only create and property changes that affect sizing invalidate measurement.
Container/style changes and native font, theme, scale, or direction changes
invalidate the affected layout. Cache frames and skip identical allocations.
An unchanged view still refreshes its current handlers but need not run layout.

## 4. Backend contract and native ownership

Keep the contract internal to the backend crates until all three implementations
prove it. Its operations are:

| Operation | Contract |
| --- | --- |
| `create` | Create a native widget for a core-assigned ID, parent, kind, and initial properties |
| `update` | Apply changed properties or ordered children of an existing parent |
| `destroy` | Disconnect and release one native widget after its children |
| `set_frame` | Apply a parent-relative logical rectangle; no implicit core layout |
| `measure` | Return native minimum/preferred sizes for known dimensions or constraints |
| `run_loop` | Own the platform loop and deliver queued events and flush requests to core |
| `post` | Wake/schedule a core flush on the main thread through a cloneable posting handle |
| `scrollbar_gutter` | Width a vertical scroll bar takes from scroll content; zero for overlay scroll bars (added in milestone 3 for Win32) |

Amended in milestone 3: a group's properties say whether its children overlap
(a stack), and `Change::Overlap` reports a switch. On Win32 one sibling order
governs both Tab order and stacking, in opposite directions, so an overlapping
host stacks later children on top at the cost of reversed Tab order inside the
stack.

Core boundary types are owned `WidgetId`, widget kind, property changes,
logical `Rect`/`Size`, measurement constraints/results, events, and errors.
They contain no HWND, Objective-C object, GTK object, or taffy public type.
Keep native handles in backend-owned storage. Internal enum payloads may grow
as widgets arrive, without exposing a backend's implementation in the app API.

Layout groups have native host containers where parenting, clipping, traversal,
or accessibility requires them. Scroll has a native viewport and a content host;
window is the native root. Backends control native ownership and representation;
core controls identity and layout. Ordered-child updates must preserve controls
and support both paint order and keyboard traversal without rebuilding survivors.

Creation, update, and measurement failures return context and the native error.
On an unrecoverable patch failure, tear down the app and return the error rather
than continuing with half-applied retained state. Cleanup tolerates already
closed native windows. Panics are not caught across native callbacks: release
builds abort as specified, while FFI boundaries must also prevent unwinding in
development builds.

Every authored `unsafe` block, including examples and benchmarks, needs an
adjacent `// SAFETY:` comment explaining its actual ownership, lifetime,
thread, pointer, or ABI justification. Prefer safe bindings when available;
do not add speculative unsafe optimizations to core.

## 5. Events, batching, and worker threads

The app controller owns state, the view factory, retained views, layout, and
handlers on the main thread. UI objects and mutable state are never `Send`.
Native callbacks enqueue an owned `(WidgetId, Event)` and request one scheduled
flush; they do not borrow application state or reconcile synchronously.

At a flush, take the pending batch, invoke current handlers in arrival order,
rerun the view once if state was touched, reconcile once, then perform invalid
layout and frame updates. Every handler in the batch sees the state changes of
the preceding handlers. Captured values come from the most recent completed
view. Do not coalesce button clicks or text edits. Events arriving during a flush
wait for the next batch; avoid an unbounded drain that starves native painting.

Store type-erased callbacks only for widgets that have handlers, using the
standard library. Native callbacks capture IDs and an event sink rather than
old application closures. Rebuilding refreshes handler registrations without
reconnecting native signals. Ignore events whose IDs have been removed.
Internal resize and appearance events can invalidate layout without rerunning
the application view. No periodic polling or permanent animation timer is needed.

Provide a UI-bound spawn facility with the conceptual shape:

```rust,ignore
ui.spawn(work, apply);
// work:  FnOnce() -> T + Send + 'static
// T:     Send + 'static
// apply: FnOnce(&mut State, T) + 'static, invoked on the UI thread
```

Use `std::thread::spawn`; retain `apply` on the UI thread and send only the owned
result and task ID through a standard-library queue. Posting wakes the native
loop, and applying results joins the same batched update path as widget events.
Results after window shutdown are discarded safely; workers do not keep the
window alive. There is no thread pool, async executor, cancellation framework,
or worker access to widgets. Exact access to the UI handle is an API question
to settle before the first example that needs background work.

## 6. Layout and platform integration

Milestone 1's temporary row/column code was replaced by taffy in milestone 2;
do not implement a second general flexbox engine. Settled semantics:

- `align` is safe: an overflowing centred or end-aligned child starts at the
  leading edge rather than being pushed past it. `Align::Stretch` fills the
  cross axis of a row or column, or a whole stack.
- A stack is as large as its largest child and places each child by `align` on
  both axes, later children drawn above earlier ones.
- A scroll viewport scrolls vertically only. Its content is laid out at the
  viewport width with unconstrained height. Its automatic minimum in the
  scrolling axis is zero.
- Native measurements are cached per leaf by constraint. A leaf is asked again
  only when a given dimension is below its natural size, e.g. for wrapping.
- The window's minimum content size is the layout's min-content size; backends
  enforce it natively.

Retain layout nodes between updates. Feed native minimum/preferred measurements
to taffy's leaf callback, honoring width-dependent measurements and native
minimum sizes. Use logical units, converting and rounding in the backend; Windows
uses 96-DPI logical units and AppKit uses points. GTK widget coordinates already
participate in GTK's scaling and must not be multiplied again. `align` controls
cross-axis child alignment in rows/columns and placement in stacks; no implicit
main-axis centering is hidden in that name.

Groups own padding and gaps; native controls retain their own theme padding.
Grow distributes available space. A scroll content tree is measured with an
unconstrained scrolling axis and the viewport constraint on the other axis.
Keep native clipping, scrolling, offsets, and focus-to-visible behavior intact.
Shrinking a viewport cannot force controls below their working native minima.

### Linux first

Use real GTK4 controls and a native window. GTK exposes minimum and natural sizes
through `Widget::measure`, including a constraint on the opposite dimension.
[GTK measurement](https://docs.gtk.org/gtk4/method.Widget.measure.html)

Use a small GTK layout host whose allocation implementation applies core-computed
child rectangles. A `GtkLayoutManager` is GTK's supported extension point for
allocation policy; `Widget::allocate` assigns child geometry and respects native
minimum sizes. `set_size_request` specifies a minimum and must not be treated as
an exact frame setter. This host arranges native controls, not their rendering.
[Layout managers](https://docs.gtk.org/gtk4/class.LayoutManager.html),
[Allocation](https://docs.gtk.org/gtk4/method.Widget.allocate.html),
[Size requests](https://docs.gtk.org/gtk4/method.Widget.set_size_request.html)

`GtkFixed` is useful for a feasibility experiment, but its documentation warns
about fonts, themes, translations, and direction changes. Hardcoded widget sizes
are therefore not the release design. Before accepting the host, test allocation
at changing window sizes, font sizes, themes, and scales, plus Tab traversal and
native accessible child order. [Fixed positioning](https://docs.gtk.org/gtk4/class.Fixed.html)

### Windows and macOS

Use standard Win32 controls and AppKit controls in milestone 3. Windows requires
per-monitor V2 awareness before window creation, handling DPI changes, and an
embedded common-controls v6 manifest. PMv2 improves native scaling but does not
remove the app's obligation to update its geometry.
[Windows DPI](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows),
[Common-controls manifest](https://learn.microsoft.com/en-us/windows/win32/controls/cookbook-overview)

Follow system appearance and accessibility settings dynamically. Verify dark
mode for each Win32 control on supported Windows versions: title-bar dark mode
alone is not proof of control theming. Do not silently use undocumented APIs or
replace native controls with owner-drawn replicas to claim parity.
[Microsoft theme guidance](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-windows-themes)

Keep keyboard navigation native, with source-order focus traversal and native
Space/Enter activation. Surviving keyed controls retain focus; removal of the
focused control follows platform traversal behavior. Preserve native accessibility
roles, values, and label relationships, including text-input labels.

Images are decoded by WIC, NSImage, or GdkTexture and displayed through native
image controls. No image-decoding crate or network loader is part of v0.1.
Propagate missing-file and decode failures. GTK's filename decoder has been
available since 4.6; its documentation restricts it to trusted input, so the
initial image contract is application-provided local assets. Broader untrusted
image support needs a separate decision about platform decoder isolation.
[WIC](https://learn.microsoft.com/en-us/windows/win32/wic/-wic-about-windows-imaging-codec),
[NSImage](https://developer.apple.com/documentation/appkit/nsimage),
[GdkTexture loading](https://docs.gtk.org/gdk4/ctor.Texture.new_from_filename.html)

## 7. Measurement and budget gates

Limits use decimal KB/MB; reports also include exact bytes. The release binary
metric is the stripped executable, with bundle/distribution bytes reported
separately. Linux GTK shared-library requirements must be listed: a small
executable alone does not establish a self-contained distribution size.

| Metric | Hard limit | Stretch goal |
| --- | --- | --- |
| Hello-world release binary | 2 MB | 800 KB |
| Hello-world idle memory | 30 MB | 15 MB |
| Overhead versus matching raw native hello world | 5 MB | 2 MB |
| Cold launch to first frame | 150 ms | 50 ms |

Milestone 1 needs a minimal hello-world probe and a matching handwritten GTK4
probe as well as the counter. Keep toolkit, renderer, title/content, window size,
theme, and build settings identical between the native probes. Add the other raw
backends when their implementations arrive. Minimal probes precede the complete
benchmark scripts and Electron comparison in milestone 4.

Memory measurement must use the requested OS metric:

- Linux: read `Pss` from `/proc/<pid>/smaps_rollup` (fall back to summing `smaps`);
  convert the kernel's kB units to bytes using 1024. Sum the application process
  tree for multiprocess applications. [Linux proc accounting](https://docs.kernel.org/filesystems/proc.html)
- Windows: private working set, not total working set or private committed bytes.
  `PROCESS_MEMORY_COUNTERS_EX2::PrivateWorkingSetSize` supplies the correct
  metric on sufficiently recent Windows. Verify an equivalent supported query
  before claiming results on older systems.
  [Windows memory counters](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex2)
- macOS: the system `footprint` tool. Verify options with the benchmark machine's
  installed manual, record the command, and collect the app plus its child
  processes together to preserve the tool's accounting of shared memory.

For an idle run, wait two seconds after the initial frame, take five samples one
second apart, and report their median and maximum. Run each native probe five
times in alternating order. Report per-probe medians and the paired differences;
never subtract measurements from different renderers, hosts, or cache conditions.
Use median idle memory and paired median overhead for the gates, retaining all
samples so transient peaks remain visible.

Time launch from an external monotonic timestamp before process creation to the
first completed client frame. `main` entry, window creation, mapping, and a
pre-paint tick are not first-frame completion. GTK's `after-paint` marks the end
of frame processing; validate a one-shot marker against actual rendering before
using it, and label compositor presentation timing separately if measured.
[GTK frame processing](https://docs.gtk.org/gdk4/signal.FrameClock.after-paint.html)

Report startup median, p95, maximum, sample count, and cache conditions. Record
at least five explicitly cold-cache launches separately from twenty fresh-process
launches with warm OS caches. A cache reset must be deliberate on a dedicated
measurement host; never clear shared-machine caches as a hidden benchmark step.
Use cold-run p95 for the startup gate. If cold conditions or the frame marker
cannot be verified, mark that metric unverified rather than claiming a pass.

Record CPU, RAM, architecture, OS, Rust and dependency versions, GTK renderer,
display server, scaling, session type, commands, raw samples, and commit/build
identity. Headless or virtual-display measurements must be labeled as such;
they do not replace tests on the intended desktop environments. Record the
instrumentation and its release-size effect separately from the shipping build.

Any hard-budget failure stops that milestone: show measured values and propose
options before proceeding. Only when a matched toolkit baseline itself makes
the absolute memory limit impossible does the overhead gate take precedence;
still show both absolute numbers. There is no exception for executable size or
startup, and no silent renderer substitution. Electron measurements in milestone
4 include its complete app process tree and distribution, with versions and
configuration disclosed; no invented comparison column.

### Milestone 0 measurements

Environment inspection on 2026-10-03 found Linux, Rust 1.98.1, Cargo 1.98.1,
and system GTK4 4.22.5. No source tree or valid Git repository was present.
These are environment observations, not application performance results.

| Application metric | Milestone 0 result |
| --- | --- |
| Release executable bytes | Not measured; no executable |
| Idle memory | Not measured; no running application |
| Framework memory overhead | Not measured; no matching probes |
| Cold start to first frame | Not measured; no running application |

## 8. Tests and milestone sequence

Core tests use a mock backend and the standard Rust test harness. The mock records
operations and supplies deterministic measurements; it requires no display.
Run focused tests for the changed behavior before any full workspace suite.

Required scenarios as the corresponding behavior arrives:

- Initial build creates the expected hierarchy; identical rebuild emits no
  native mutations; changed properties patch only those properties.
- Kind replacement and subtree removal destroy children safely, unregister
  handlers, and release layout nodes; stale events are ignored.
- Keyed insertion, removal, and reordering preserve surviving identities and
  update child order; duplicate keys fail clearly.
- Handler replacement invokes the latest closure without reconnecting native
  callbacks; two queued clicks mutate state twice and trigger one view rebuild.
- An accepted native edit produces no redundant setter; programmatic changes
  do not feed back into handlers; caret/IME retention is checked natively.
- Native measurement constraints reach the backend, only invalid nodes are
  remeasured, and unchanged computed frames are skipped. Verify gap, padding,
  grow, align, stack ordering, scroll constraints, resize, and scale changes.
- Worker results mutate state only on the UI thread, share batching behavior,
  and safely disappear after shutdown. Failure cleanup leaves no live handlers.

Native acceptance adds keyboard-only operation, accessibility inspection, IME
and selection, font/theme/scale changes, scrolling, and valid/invalid image
loads. Check the widgets actually used; mock tests cannot prove OS integration.

| Milestone | Deliverable and stop point |
| --- | --- |
| 0 | This architecture document. Stop and wait for approval. |
| 1 | Present the implementation plan first. Build Linux window/text/button and minimal counter layout; run counter, focused mock tests, and native hello-world measurements. Report and stop. |
| 2 | Present the plan first. Add taffy and remaining Linux widgets, keyed todo collection, todo and settings-form examples. Repeat the hello-world gates, report dependency deltas and example results, then stop. |
| 3 | Present the plan first. Add Win32 and AppKit; counter, todo, and settings form run from identical Rust sources. Test and measure on each actual OS, report and stop. |
| 4 | Present the plan first. Add `bench/` automation and raw-native/Electron probes; publish reproducible results and the comparison table in README. Report and stop. |

Keep implementation commits small and reviewable, avoid unrelated edits, and
use built-in testing rather than adding a test framework. Milestone 0 creates
only this document; repository initialization and commits await an actual
writable Git repository. Runtime code and dependency declarations begin only
after the milestone 0 go-ahead.

## 9. Open questions and experiments

These are explicit implementation gates, not promised solved platform behavior:

| Question | Resolution point |
| --- | --- |
| Can the GTK allocation host apply exact core frames without conflicting allocations, resize loops, or focus/accessibility regressions? | Prototype and validate before accepting milestone 1; extend to scroll in milestone 2. |
| Does GTK's default renderer make absolute idle memory or startup unattainable here? | Matched done/raw probes in milestone 1; report budgets before proceeding. |
| Which first-frame signal reliably describes the completed initial content on each OS? | Validate against rendering in that backend's first measurement; leave unsupported timings unverified. |
| What minimum Rust/toolkit/OS versions can we actually support? | Resolve the smallest binding features and test their minimum versions; document rather than infer compatibility from this machine. |
| How should handlers obtain the UI posting/spawn handle while retaining the simple `&mut State` API? | Propose the smallest context extension before implementing threading; no global state or state-owned native handle. |
| How do native composition and external model changes interact? | Native text-input tests in milestone 2; do not blindly replace the value during active IME composition. |
| Which documented Win32 APIs theme every requested native control in dark mode and high contrast? | Per-control proof in milestone 3. Report any conflict with the native-widget requirement before implementation compromises. |
| How is the executable manifest reliably embedded for consuming Rust apps on each supported Windows toolchain? | Verify final executable resources in milestone 3; a manifest sitting beside a backend crate is insufficient. |
| Where will Windows/macOS native tests and measurements run? | Arrange actual OS runners for milestone 3; cross-compilation alone cannot establish runtime results. |

Milestone 0 ends here. Approval authorizes planning milestone 1; every subsequent
milestone still has its own plan, measurements, report, and stop point.

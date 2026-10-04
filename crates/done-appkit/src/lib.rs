//! AppKit implementation of done's native-widget contract: standard AppKit
//! controls in flipped host views, driven by `NSApplication`.
//!
//! Compiled and linted for macOS targets; see the milestone 3 report for what
//! has not yet run on macOS.
#![cfg(target_os = "macos")]
#![deny(clippy::undocumented_unsafe_blocks)]

use done_core::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton,
    NSControl, NSControlSize, NSControlStateValueOff, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSEvent, NSEventModifierFlags, NSEventType, NSFont, NSImage,
    NSImageView, NSPopUpButton, NSProgressIndicator, NSProgressIndicatorStyle, NSResponder,
    NSScrollView, NSScroller, NSScrollerStyle, NSSlider, NSTextField, NSTextFieldDelegate, NSView,
    NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSNotification, NSPoint, NSRect, NSRunLoopCommonModes, NSSize, NSString,
};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    ptr,
    rc::Rc,
};

type Dispatch = Box<dyn FnMut(Vec<Event>)>;

/// Native callbacks only enqueue; one batch runs per scheduled flush.
#[derive(Default)]
struct Queue {
    pending: RefCell<Vec<Event>>,
    scheduled: Cell<bool>,
    dispatch: RefCell<Option<Dispatch>>,
    target: RefCell<Option<Retained<Target>>>,
}

impl Queue {
    fn push(&self, event: Event) {
        self.pending.borrow_mut().push(event);
        if !self.scheduled.replace(true) {
            self.schedule();
        }
    }

    fn schedule(&self) {
        if let Some(target) = self.target.borrow().as_ref() {
            // SAFETY: the extern static is a constant run-loop mode string.
            let modes = NSArray::from_slice(&[unsafe { NSRunLoopCommonModes }]);
            // SAFETY: `flush:` is implemented by `Target`, takes one object
            // argument (nil here), and the call happens on the main thread.
            // Common modes keep batches flowing during tracking, e.g. while
            // a slider is dragged.
            let _: () = unsafe {
                msg_send![
                    &**target,
                    performSelector: sel!(flush:),
                    withObject: ptr::null::<AnyObject>(),
                    afterDelay: 0.0f64,
                    inModes: &*modes
                ]
            };
        }
    }

    fn flush(&self) {
        self.scheduled.set(false);
        let events = std::mem::take(&mut *self.pending.borrow_mut());
        if events.is_empty() {
            return;
        }
        // Taken out while it runs: native calls inside may deliver callbacks.
        let dispatch = self.dispatch.borrow_mut().take();
        match dispatch {
            Some(mut dispatch) => {
                dispatch(events);
                let mut slot = self.dispatch.borrow_mut();
                if slot.is_none() {
                    *slot = Some(dispatch);
                }
            }
            None => {
                let mut pending = self.pending.borrow_mut();
                let later = std::mem::replace(&mut *pending, events);
                pending.extend(later);
            }
        }
        if !self.pending.borrow().is_empty() && !self.scheduled.replace(true) {
            self.schedule();
        }
    }
}

fn widget_id(control: &NSControl) -> WidgetId {
    WidgetId(control.tag() as u64)
}

fn ns(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

struct TargetIvars {
    queue: Rc<Queue>,
    sliders: RefCell<HashMap<isize, SliderRange>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Target` does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DoneTarget"]
    #[ivars = TargetIvars]
    struct Target;

    impl Target {
        #[unsafe(method(click:))]
        fn click(&self, sender: &NSControl) {
            self.ivars().queue.push(Event::Click(widget_id(sender)));
        }

        #[unsafe(method(toggle:))]
        fn toggle(&self, sender: &NSButton) {
            let checked = sender.state() == NSControlStateValueOn;
            self.ivars()
                .queue
                .push(Event::Toggle(widget_id(sender), checked));
        }

        #[unsafe(method(slide:))]
        fn slide(&self, sender: &NSSlider) {
            let mut value = sender.doubleValue();
            // The native slider is continuous; report values on the step grid.
            if let Some(range) = self.ivars().sliders.borrow().get(&sender.tag()) {
                value = (range.min + ((value - range.min) / range.step).round() * range.step)
                    .clamp(range.min, range.max);
            }
            self.ivars().queue.push(Event::Slide(widget_id(sender), value));
        }

        #[unsafe(method(select:))]
        fn select(&self, sender: &NSPopUpButton) {
            let index = sender.indexOfSelectedItem();
            if index >= 0 {
                self.ivars()
                    .queue
                    .push(Event::Select(widget_id(sender), index as usize));
            }
        }

        #[unsafe(method(submit:))]
        fn submit(&self, sender: &NSTextField) {
            self.ivars().queue.push(Event::Submit(widget_id(sender)));
        }

        #[unsafe(method(flush:))]
        fn flush(&self, _: Option<&AnyObject>) {
            self.ivars().queue.flush();
        }
    }

    unsafe impl NSObjectProtocol for Target {}

    unsafe impl NSControlTextEditingDelegate for Target {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            let field = notification
                .object()
                .and_then(|object| object.downcast::<NSTextField>().ok());
            if let Some(field) = field {
                let text = field.stringValue().to_string();
                self.ivars().queue.push(Event::Input(widget_id(&field), text));
            }
        }
    }

    unsafe impl NSTextFieldDelegate for Target {}

    unsafe impl NSWindowDelegate for Target {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _: &NSWindow) -> bool {
            // Core tears the window down; never close behind its back.
            self.ivars().queue.push(Event::Close);
            false
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn backing_changed(&self, _: &NSNotification) {
            self.ivars().queue.push(Event::AppearanceChanged);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, queue: Rc<Queue>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars {
            queue,
            sliders: RefCell::new(HashMap::new()),
        });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

struct HostIvars {
    /// The window's content view reports resizes and appearance changes.
    root: Option<Rc<Queue>>,
    last: Cell<Option<(f64, f64)>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super in
    // the overridden methods, which both overrides do; no Drop.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DoneHostView"]
    #[ivars = HostIvars]
    struct Host;

    impl Host {
        /// Top-left origin, like core's frames.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: forwarding to NSView with the same argument.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if let Some(queue) = &self.ivars().root {
                let size_pair = (size.width, size.height);
                if self.ivars().last.replace(Some(size_pair)) != Some(size_pair) {
                    queue.push(Event::Resized(Size {
                        width: size.width as f32,
                        height: size.height as f32,
                    }));
                }
            }
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            // SAFETY: forwarding to NSView.
            let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
            if let Some(queue) = &self.ivars().root {
                queue.push(Event::AppearanceChanged);
            }
        }
    }
);

impl Host {
    fn new(mtm: MainThreadMarker, root: Option<Rc<Queue>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HostIvars {
            root,
            last: Cell::new(None),
        });
        let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
        // SAFETY: NSView's designated initializer.
        unsafe { msg_send![super(this), initWithFrame: zero] }
    }
}

struct Native {
    view: Retained<NSView>,
    kind: Kind,
    /// Where this widget's children are placed.
    host: Option<Retained<NSView>>,
    parent: Option<WidgetId>,
    /// Scroll viewports: frames of direct children, for the content extent.
    frames: HashMap<WidgetId, Rect>,
}

pub struct AppKitBackend {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    target: Retained<Target>,
    queue: Rc<Queue>,
    widgets: HashMap<WidgetId, Native>,
    window: Option<Retained<NSWindow>>,
    root: Option<Retained<Host>>,
    /// Structure changed since the key view loop was last rebuilt.
    keys_dirty: bool,
}

fn zero_rect() -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0))
}

fn options_array(options: &[Cow<'static, str>]) -> Retained<NSArray<NSString>> {
    let options: Vec<Retained<NSString>> = options.iter().map(|option| ns(option)).collect();
    NSArray::from_retained_slice(&options)
}

impl AppKitBackend {
    fn native(&self, id: WidgetId) -> Result<&Native> {
        self.widgets
            .get(&id)
            .ok_or_else(|| format!("unknown widget {}", id.0).into())
    }

    fn get<T: objc2::DowncastTarget>(&self, id: WidgetId) -> Result<&T> {
        let object: &AnyObject = &self.native(id)?.view;
        object
            .downcast_ref::<T>()
            .ok_or_else(|| format!("widget {} has an unexpected native class", id.0).into())
    }

    fn label(view: &NSView, label: &str) {
        let label = (!label.is_empty()).then(|| ns(label));
        view.setAccessibilityLabel(label.as_deref());
    }

    fn create_window(&mut self, options: &WindowOptions) -> Retained<NSView> {
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(options.size.width as f64, options.size.height as f64),
        );
        // SAFETY: NSWindow's designated initializer on the main thread.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                self.mtm.alloc(),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window is owned by this backend, not released on close.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&ns(&options.title));
        let root = Host::new(self.mtm, Some(self.queue.clone()));
        window.setContentView(Some(&root));
        window.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
        // Source-order Tab traversal is maintained explicitly.
        window.setAutorecalculatesKeyViewLoop(false);
        window.center();
        let view: Retained<NSView> = root.clone().into_super();
        self.root = Some(root);
        self.window = Some(window);
        view
    }

    /// Chain focusable controls in source order, wrapping around.
    fn refresh_key_loop(&mut self) {
        if !std::mem::take(&mut self.keys_dirty) {
            return;
        }
        let (Some(window), Some(root)) = (&self.window, &self.root) else {
            return;
        };
        let mut chain: Vec<Retained<NSView>> = Vec::new();
        fn collect(view: &NSView, chain: &mut Vec<Retained<NSView>>) {
            for child in view.subviews().iter() {
                let object: &AnyObject = &child;
                if let Some(scroll) = object.downcast_ref::<NSScrollView>() {
                    if let Some(content) = scroll.documentView() {
                        collect(&content, chain);
                    }
                } else if object.downcast_ref::<Host>().is_some() {
                    collect(&child, chain);
                } else if let Some(field) = object.downcast_ref::<NSTextField>() {
                    if field.isEditable() {
                        chain.push(child.clone());
                    }
                } else if object.downcast_ref::<NSControl>().is_some() {
                    chain.push(child.clone());
                }
            }
        }
        collect(root, &mut chain);
        for (index, view) in chain.iter().enumerate() {
            let next = &chain[(index + 1) % chain.len()];
            // SAFETY: both views are live in this window; a loop is the
            // intended key-view structure.
            unsafe { view.setNextKeyView(Some(next)) };
        }
        window.setInitialFirstResponder(chain.first().map(|view| &**view));
    }

    fn update_extent(&mut self, scroll: WidgetId) {
        let Some(native) = self.widgets.get(&scroll) else {
            return;
        };
        let (width, height) =
            native
                .frames
                .values()
                .fold((0.0f32, 0.0f32), |(width, height), rect| {
                    (
                        width.max(rect.x + rect.width),
                        height.max(rect.y + rect.height),
                    )
                });
        if let Some(content) = &native.host {
            content.setFrameSize(NSSize::new(width as f64, height as f64));
        }
    }

    fn stop(&self) {
        self.app.stop(None);
        // `stop:` takes effect after the next event; post one to wake the loop.
        let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            0,
            0,
        );
        if let Some(event) = event {
            self.app.postEvent_atStart(&event, true);
        }
    }
}

impl Backend for AppKitBackend {
    fn create(
        &mut self,
        id: WidgetId,
        parent: Option<WidgetId>,
        properties: Properties<'_>,
    ) -> Result<()> {
        let mtm = self.mtm;
        let target: &AnyObject = &self.target;
        let tag = id.0 as isize;
        let mut host = None;
        let view: Retained<NSView> = match properties {
            Properties::Window(options) => {
                let root = self.create_window(options);
                self.widgets.insert(
                    id,
                    Native {
                        view: root.clone(),
                        kind: Kind::Window,
                        host: Some(root),
                        parent: None,
                        frames: HashMap::new(),
                    },
                );
                return Ok(());
            }
            // AppKit draws later subviews on top, so overlap needs nothing.
            Properties::Group { .. } => {
                let group: Retained<NSView> = Host::new(mtm, None).into_super();
                host = Some(group.clone());
                group
            }
            Properties::Scroll => {
                let scroll = NSScrollView::initWithFrame(mtm.alloc(), zero_rect());
                scroll.setHasVerticalScroller(true);
                scroll.setAutohidesScrollers(true);
                scroll.setDrawsBackground(false);
                let content: Retained<NSView> = Host::new(mtm, None).into_super();
                scroll.setDocumentView(Some(&content));
                host = Some(content);
                scroll.into_super()
            }
            Properties::Text { value, size } => {
                let label = NSTextField::labelWithString(&ns(value), mtm);
                if let Some(size) = size {
                    label.setFont(Some(&NSFont::systemFontOfSize(size as f64)));
                }
                label.into_super().into_super()
            }
            Properties::Button(label) => {
                // SAFETY: `target` implements `click:` and outlives the button.
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &ns(label),
                        Some(target),
                        Some(sel!(click:)),
                        mtm,
                    )
                };
                button.setTag(tag);
                button.into_super().into_super()
            }
            Properties::TextInput {
                value,
                placeholder,
                label,
            } => {
                let field = NSTextField::textFieldWithString(&ns(value), mtm);
                field.setTag(tag);
                if !placeholder.is_empty() {
                    field.setPlaceholderString(Some(&ns(placeholder)));
                }
                // Submit means Return, not focus leaving the field.
                if let Some(cell) = field.cell() {
                    cell.setSendsActionOnEndEditing(false);
                }
                // SAFETY: the target implements the delegate methods and
                // `submit:`, and outlives the field.
                unsafe {
                    field.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                    field.setTarget(Some(target));
                    field.setAction(Some(sel!(submit:)));
                }
                Self::label(&field, label);
                field.into_super().into_super()
            }
            Properties::Checkbox { label, checked } => {
                // SAFETY: `target` implements `toggle:` and outlives the button.
                let check = unsafe {
                    NSButton::checkboxWithTitle_target_action(
                        &ns(label),
                        Some(target),
                        Some(sel!(toggle:)),
                        mtm,
                    )
                };
                check.setTag(tag);
                check.setState(if checked {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                check.into_super().into_super()
            }
            Properties::Slider {
                value,
                range,
                label,
            } => {
                // SAFETY: `target` implements `slide:` and outlives the slider.
                let slider = unsafe {
                    NSSlider::sliderWithValue_minValue_maxValue_target_action(
                        value,
                        range.min,
                        range.max,
                        Some(target),
                        Some(sel!(slide:)),
                        mtm,
                    )
                };
                slider.setTag(tag);
                slider.setContinuous(true);
                self.target.ivars().sliders.borrow_mut().insert(tag, range);
                Self::label(&slider, label);
                slider.into_super().into_super()
            }
            Properties::Progress { fraction, label } => {
                let bar = NSProgressIndicator::initWithFrame(mtm.alloc(), zero_rect());
                bar.setStyle(NSProgressIndicatorStyle::Bar);
                bar.setIndeterminate(false);
                bar.setMinValue(0.0);
                bar.setMaxValue(1.0);
                bar.setDoubleValue(fraction);
                Self::label(&bar, label);
                bar.into_super()
            }
            Properties::Dropdown {
                options,
                selected,
                label,
            } => {
                let popup = NSPopUpButton::initWithFrame_pullsDown(mtm.alloc(), zero_rect(), false);
                popup.addItemsWithTitles(&options_array(options));
                popup.selectItemAtIndex(selected.map_or(-1, |index| index as isize));
                popup.setTag(tag);
                // SAFETY: `target` implements `select:` and outlives the button.
                unsafe {
                    popup.setTarget(Some(target));
                    popup.setAction(Some(sel!(select:)));
                }
                Self::label(&popup, label);
                popup.into_super().into_super().into_super()
            }
            Properties::Image { path, label } => {
                let file = ns(&path.to_string_lossy());
                let image =
                    NSImage::initWithContentsOfFile(mtm.alloc(), &file).ok_or_else(|| {
                        format!("{}: missing or not a decodable image", path.display())
                    })?;
                let view = NSImageView::imageViewWithImage(&image, mtm);
                Self::label(&view, label);
                view.into_super().into_super()
            }
        };
        let parent = parent.ok_or("a control needs a parent")?;
        let parent_host = self
            .native(parent)?
            .host
            .clone()
            .ok_or_else(|| format!("widget {} cannot have children", parent.0))?;
        parent_host.addSubview(&view);
        self.widgets.insert(
            id,
            Native {
                view,
                kind: properties.kind(),
                host,
                parent: Some(parent),
                frames: HashMap::new(),
            },
        );
        self.keys_dirty = true;
        Ok(())
    }

    fn update(&mut self, id: WidgetId, change: Change<'_>) -> Result<()> {
        // AppKit setters do not send actions or text-change notifications,
        // so programmatic changes never echo back as edits.
        match change {
            Change::Text(value) | Change::Value(value) => {
                self.get::<NSControl>(id)?.setStringValue(&ns(value))
            }
            Change::TextSize(size) => {
                let font = match size {
                    Some(size) => NSFont::systemFontOfSize(size as f64),
                    None => NSFont::systemFontOfSize(NSFont::systemFontSize()),
                };
                self.get::<NSControl>(id)?.setFont(Some(&font));
            }
            Change::Label(value) => self.get::<NSButton>(id)?.setTitle(&ns(value)),
            Change::AccessibleLabel(value) => Self::label(&self.native(id)?.view, value),
            Change::Placeholder(value) => self
                .get::<NSTextField>(id)?
                .setPlaceholderString((!value.is_empty()).then(|| ns(value)).as_deref()),
            Change::Checked(checked) => self.get::<NSButton>(id)?.setState(if checked {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            }),
            Change::SliderValue(value) => self.get::<NSSlider>(id)?.setDoubleValue(value),
            Change::SliderRange(range) => {
                let slider = self.get::<NSSlider>(id)?;
                slider.setMinValue(range.min);
                slider.setMaxValue(range.max);
                self.target
                    .ivars()
                    .sliders
                    .borrow_mut()
                    .insert(id.0 as isize, range);
            }
            Change::Fraction(fraction) => self
                .get::<NSProgressIndicator>(id)?
                .setDoubleValue(fraction),
            Change::Options(options) => {
                let popup = self.get::<NSPopUpButton>(id)?;
                popup.removeAllItems();
                popup.addItemsWithTitles(&options_array(options));
            }
            Change::Selected(selected) => self
                .get::<NSPopUpButton>(id)?
                .selectItemAtIndex(selected.map_or(-1, |index| index as isize)),
            Change::Image(path) => {
                let file = ns(&path.to_string_lossy());
                let image =
                    NSImage::initWithContentsOfFile(self.mtm.alloc(), &file).ok_or_else(|| {
                        format!("{}: missing or not a decodable image", path.display())
                    })?;
                self.get::<NSImageView>(id)?.setImage(Some(&image));
            }
            Change::Order(order) => {
                let host = self.native(id)?.host.clone().ok_or("not a container")?;
                let views = order
                    .iter()
                    .map(|child| Ok(self.native(*child)?.view.clone()))
                    .collect::<Result<Vec<_>>>()?;
                // Reorders existing subviews; later ones are drawn on top.
                host.setSubviews(&NSArray::from_retained_slice(&views));
                self.keys_dirty = true;
            }
            Change::MinimumSize(size) => {
                if let Some(window) = &self.window {
                    window.setContentMinSize(NSSize::new(size.width as f64, size.height as f64));
                }
            }
            Change::Overlap(_) => {}
        }
        Ok(())
    }

    fn destroy(&mut self, id: WidgetId) {
        let Some(native) = self.widgets.remove(&id) else {
            return;
        };
        if native.kind == Kind::Window {
            if let Some(window) = self.window.take() {
                window.close();
            }
            self.root = None;
            self.stop();
            return;
        }
        native.view.removeFromSuperview();
        if native.kind == Kind::Slider {
            self.target
                .ivars()
                .sliders
                .borrow_mut()
                .remove(&(id.0 as isize));
        }
        if let Some(parent) = native.parent {
            let is_scroll = self
                .widgets
                .get_mut(&parent)
                .filter(|parent| parent.kind == Kind::Scroll)
                .map(|parent| parent.frames.remove(&id))
                .is_some();
            if is_scroll {
                self.update_extent(parent);
            }
        }
        self.keys_dirty = true;
    }

    fn set_frame(&mut self, id: WidgetId, rect: Rect) -> Result<()> {
        let native = self.native(id)?;
        // Round edges, not sizes, so siblings never gap or overlap.
        let left = rect.x.round() as f64;
        let top = rect.y.round() as f64;
        let right = (rect.x + rect.width).round() as f64;
        let bottom = (rect.y + rect.height).round() as f64;
        native.view.setFrame(NSRect::new(
            NSPoint::new(left, top),
            NSSize::new(right - left, bottom - top),
        ));
        if let Some(parent) = native.parent {
            if let Some(scroll) = self
                .widgets
                .get_mut(&parent)
                .filter(|parent| parent.kind == Kind::Scroll)
            {
                scroll.frames.insert(id, rect);
                self.update_extent(parent);
            }
        }
        Ok(())
    }

    fn measure(&mut self, id: WidgetId, _: Constraints) -> Result<Measurement> {
        let native = self.native(id)?;
        let fitting = native.view.fittingSize();
        let (width, height) = (fitting.width as f32, fitting.height as f32);
        let (minimum, preferred) = match native.kind {
            Kind::TextInput => ((100.0, height), (200.0, height)),
            Kind::Slider => ((80.0, height.max(21.0)), (160.0, height.max(21.0))),
            Kind::Progress => ((40.0, height.max(20.0)), (160.0, height.max(20.0))),
            Kind::Window | Kind::Group | Kind::Scroll => ((0.0, 0.0), (0.0, 0.0)),
            _ => ((width, height), (width, height)),
        };
        let size = |(width, height): (f32, f32)| Size { width, height };
        Ok(Measurement {
            minimum: size(minimum),
            preferred: size(preferred),
        })
    }

    fn scrollbar_gutter(&self) -> f32 {
        // Overlay scrollers take no space; legacy ones do.
        if NSScroller::preferredScrollerStyle(self.mtm) == NSScrollerStyle::Legacy {
            NSScroller::scrollerWidthForControlSize_scrollerStyle(
                NSControlSize::Regular,
                NSScrollerStyle::Legacy,
                self.mtm,
            ) as f32
        } else {
            0.0
        }
    }

    fn post(&self, event: Event) {
        self.queue.push(event);
    }

    fn run_loop(
        mut dispatch: impl FnMut(&mut Self, &[Event]) -> Result<()> + 'static,
    ) -> Result<()> {
        let mtm = MainThreadMarker::new().ok_or("done must run on the main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let queue = Rc::new(Queue::default());
        let target = Target::new(mtm, queue.clone());
        *queue.target.borrow_mut() = Some(target.clone());
        let mut backend = Self {
            mtm,
            app: app.clone(),
            target,
            queue: queue.clone(),
            widgets: HashMap::new(),
            window: None,
            root: None,
            keys_dirty: false,
        };
        let started = dispatch(&mut backend, &[Event::Started]);
        if let Err(error) = started {
            queue.target.borrow_mut().take();
            return Err(error);
        }
        backend.refresh_key_loop();
        let window = backend
            .window
            .clone()
            .ok_or("app did not create a window")?;
        let failure = Rc::new(RefCell::new(None));
        let observed = failure.clone();
        let pair = Rc::new(RefCell::new((backend, dispatch)));
        let weak = Rc::downgrade(&pair);
        *queue.dispatch.borrow_mut() = Some(Box::new(move |events: Vec<Event>| {
            if let Some(pair) = weak.upgrade() {
                let mut pair = pair.borrow_mut();
                let (backend, dispatch) = &mut *pair;
                match dispatch(backend, &events) {
                    Ok(()) => backend.refresh_key_loop(),
                    Err(error) => {
                        *observed.borrow_mut() = Some(error);
                        backend.stop();
                    }
                }
            }
        }));
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        app.run();
        queue.dispatch.borrow_mut().take();
        queue.pending.borrow_mut().clear();
        // Break the target-queue reference cycle.
        queue.target.borrow_mut().take();
        drop(pair);
        let result = match failure.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        };
        result
    }
}

impl Drop for AppKitBackend {
    fn drop(&mut self) {
        if let Some(window) = self.window.take() {
            window.close();
        }
        self.widgets.clear();
    }
}

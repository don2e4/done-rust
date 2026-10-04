//! GTK4 implementation of done's native-widget contract.
#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

mod host;
#[cfg(feature = "probe")]
pub mod probe;

use done_core::*;
use gtk4::{self as gtk, gdk, gio, glib, prelude::*};
use host::{Host, Role};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::Rc,
};

type Dispatch = Box<dyn FnMut(Vec<Event>)>;

#[derive(Default)]
struct Queue {
    pending: RefCell<Vec<Event>>,
    scheduled: Cell<bool>,
    dispatch: RefCell<Option<Dispatch>>,
}

impl Queue {
    fn push(self: &Rc<Self>, event: Event) {
        self.pending.borrow_mut().push(event);
        if !self.scheduled.replace(true) {
            let queue = self.clone();
            glib::idle_add_local_once(move || {
                abort_on_panic(|| {
                    queue.scheduled.set(false);
                    let events = std::mem::take(&mut *queue.pending.borrow_mut());
                    if let Some(dispatch) = queue.dispatch.borrow_mut().as_mut() {
                        dispatch(events);
                    }
                });
            });
        }
    }
}

// Native callbacks must never unwind into C, including development builds.
fn abort_on_panic<T>(f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| std::process::abort())
}

struct Native {
    widget: gtk::Widget,
    signals: Vec<glib::SignalHandlerId>,
    /// The value-change signal, blocked around programmatic changes so they
    /// are never reported back as user edits.
    guarded: Option<glib::SignalHandlerId>,
    /// Where this widget's children are placed.
    host: Option<Host>,
}

pub struct GtkBackend {
    widgets: HashMap<WidgetId, Native>,
    queue: Rc<Queue>,
    main_loop: glib::MainLoop,
    settings_signals: Vec<(gtk::Settings, glib::SignalHandlerId)>,
}

fn font_size(label: &gtk::Label, size: Option<f32>) {
    let attrs = size.map(|size| {
        let attrs = gtk::pango::AttrList::new();
        attrs.insert(gtk::pango::AttrSize::new_size_absolute(
            (size * gtk::pango::SCALE as f32).round() as i32,
        ));
        attrs
    });
    label.set_attributes(attrs.as_ref());
}

fn accessible_label(widget: &impl IsA<gtk::Accessible>, label: &str) {
    if label.is_empty() {
        widget.reset_property(gtk::AccessibleProperty::Label);
    } else {
        widget.update_property(&[gtk::accessible::Property::Label(label)]);
    }
}

fn texture(path: &Path) -> Result<gdk::Texture> {
    gdk::Texture::from_file(&gio::File::for_path(path))
        .map_err(|error| format!("{}: {error}", path.display()).into())
}

fn list_position(index: Option<usize>) -> u32 {
    index
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(gtk::INVALID_LIST_POSITION)
}

fn string_list(options: &[Cow<'static, str>]) -> gtk::StringList {
    let options: Vec<&str> = options.iter().map(AsRef::as_ref).collect();
    gtk::StringList::new(&options)
}

impl GtkBackend {
    fn native(&self, id: WidgetId) -> Result<&Native> {
        self.widgets
            .get(&id)
            .ok_or_else(|| format!("unknown widget {}", id.0).into())
    }

    fn get<T: IsA<gtk::Widget>>(&self, id: WidgetId) -> Result<T> {
        self.native(id)?
            .widget
            .clone()
            .downcast::<T>()
            .map_err(|widget| {
                format!(
                    "widget {} is a {}, not a {}",
                    id.0,
                    widget.type_().name(),
                    T::static_type().name()
                )
                .into()
            })
    }

    fn host(&self, id: WidgetId) -> Result<&Host> {
        self.native(id)?
            .host
            .as_ref()
            .ok_or_else(|| format!("widget {} cannot have children", id.0).into())
    }

    /// Apply a programmatic change without reporting it as a user edit.
    fn guarded(&self, id: WidgetId, change: impl FnOnce()) -> Result<()> {
        let native = self.native(id)?;
        if let Some(signal) = &native.guarded {
            native.widget.block_signal(signal);
        }
        change();
        if let Some(signal) = &native.guarded {
            native.widget.unblock_signal(signal);
        }
        Ok(())
    }

    fn window(&self, options: &WindowOptions) -> Native {
        let window = gtk::Window::builder()
            .title(options.title.as_ref())
            .default_width(options.size.width.round() as i32)
            .default_height(options.size.height.round() as i32)
            .build();
        let root = Host::new(Role::Root);
        root.set_queue(self.queue.clone());
        window.set_child(Some(&root));
        let mut signals = Vec::new();
        let queue = self.queue.clone();
        signals.push(window.connect_close_request(move |_| {
            abort_on_panic(|| queue.push(Event::Close));
            glib::Propagation::Stop
        }));
        let queue = self.queue.clone();
        signals.push(
            window.connect_notify_local(Some("scale-factor"), move |_, _| {
                abort_on_panic(|| queue.push(Event::AppearanceChanged));
            }),
        );
        #[cfg(feature = "probe")]
        probe::install(&window);
        Native {
            widget: window.upcast(),
            signals,
            guarded: None,
            host: Some(root),
        }
    }
}

impl Backend for GtkBackend {
    fn create(
        &mut self,
        id: WidgetId,
        parent: Option<WidgetId>,
        properties: Properties<'_>,
    ) -> Result<()> {
        let queue = self.queue.clone();
        let mut signals = Vec::new();
        let mut guarded = None;
        let mut host = None;
        let widget: gtk::Widget = match properties {
            Properties::Window(options) => {
                let native = self.window(options);
                self.widgets.insert(id, native);
                return Ok(());
            }
            // GTK draws children in order, so overlap needs no special case.
            Properties::Group { .. } => {
                let group = Host::new(Role::Group);
                host = Some(group.clone());
                group.upcast()
            }
            Properties::Scroll => {
                let content = Host::new(Role::Content);
                let scroll = gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .vscrollbar_policy(gtk::PolicyType::Automatic)
                    .child(&content)
                    .build();
                host = Some(content);
                scroll.upcast()
            }
            Properties::Text { value, size } => {
                let label = gtk::Label::new(Some(value));
                label.set_xalign(0.0);
                label.set_yalign(0.0);
                font_size(&label, size);
                label.upcast()
            }
            Properties::Button(label) => {
                let button = gtk::Button::with_label(label);
                signals.push(
                    button
                        .connect_clicked(move |_| abort_on_panic(|| queue.push(Event::Click(id)))),
                );
                button.upcast()
            }
            Properties::TextInput {
                value,
                placeholder,
                label,
            } => {
                let entry = gtk::Entry::new();
                entry.set_text(value);
                if !placeholder.is_empty() {
                    entry.set_placeholder_text(Some(placeholder));
                }
                accessible_label(&entry, label);
                let changed = queue.clone();
                guarded = Some(entry.connect_changed(move |entry| {
                    abort_on_panic(|| changed.push(Event::Input(id, entry.text().into())))
                }));
                signals.push(
                    entry.connect_activate(move |_| {
                        abort_on_panic(|| queue.push(Event::Submit(id)))
                    }),
                );
                entry.upcast()
            }
            Properties::Checkbox { label, checked } => {
                let check = gtk::CheckButton::with_label(label);
                check.set_active(checked);
                guarded = Some(check.connect_toggled(move |check| {
                    abort_on_panic(|| queue.push(Event::Toggle(id, check.is_active())))
                }));
                check.upcast()
            }
            Properties::Slider {
                value,
                range,
                label,
            } => {
                let scale = gtk::Scale::with_range(
                    gtk::Orientation::Horizontal,
                    range.min,
                    range.max,
                    range.step,
                );
                scale.set_draw_value(false);
                scale.set_value(value);
                accessible_label(&scale, label);
                guarded = Some(scale.connect_value_changed(move |scale| {
                    abort_on_panic(|| queue.push(Event::Slide(id, scale.value())))
                }));
                scale.upcast()
            }
            Properties::Progress { fraction, label } => {
                let bar = gtk::ProgressBar::new();
                bar.set_fraction(fraction);
                accessible_label(&bar, label);
                bar.upcast()
            }
            Properties::Dropdown {
                options,
                selected,
                label,
            } => {
                let dropdown =
                    gtk::DropDown::new(Some(string_list(options)), None::<gtk::Expression>);
                dropdown.set_selected(list_position(selected));
                accessible_label(&dropdown, label);
                guarded = Some(dropdown.connect_selected_notify(move |dropdown| {
                    let selected = dropdown.selected();
                    if selected != gtk::INVALID_LIST_POSITION {
                        abort_on_panic(|| queue.push(Event::Select(id, selected as usize)));
                    }
                }));
                dropdown.upcast()
            }
            Properties::Image { path, label } => {
                let picture = gtk::Picture::for_paintable(&texture(path)?);
                picture.set_can_shrink(false);
                if !label.is_empty() {
                    picture.set_alternative_text(Some(label));
                }
                picture.upcast()
            }
        };
        if let Some(parent) = parent {
            self.host(parent)?.add(&widget);
        }
        self.widgets.insert(
            id,
            Native {
                widget,
                signals,
                guarded,
                host,
            },
        );
        Ok(())
    }

    fn update(&mut self, id: WidgetId, change: Change<'_>) -> Result<()> {
        match change {
            Change::Text(value) => self.get::<gtk::Label>(id)?.set_text(value),
            Change::TextSize(size) => font_size(&self.get::<gtk::Label>(id)?, size),
            Change::Label(value) => {
                let widget = &self.native(id)?.widget;
                if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                    button.set_label(value);
                } else {
                    self.get::<gtk::CheckButton>(id)?.set_label(Some(value));
                }
            }
            Change::AccessibleLabel(value) => {
                let widget = &self.native(id)?.widget;
                match widget.downcast_ref::<gtk::Picture>() {
                    Some(picture) => {
                        picture.set_alternative_text((!value.is_empty()).then_some(value))
                    }
                    None => accessible_label(widget, value),
                }
            }
            Change::Value(value) => {
                let entry = self.get::<gtk::Entry>(id)?;
                self.guarded(id, || entry.set_text(value))?;
            }
            Change::Placeholder(value) => self
                .get::<gtk::Entry>(id)?
                .set_placeholder_text((!value.is_empty()).then_some(value)),
            Change::Checked(checked) => {
                let check = self.get::<gtk::CheckButton>(id)?;
                self.guarded(id, || check.set_active(checked))?;
            }
            Change::SliderValue(value) => {
                let scale = self.get::<gtk::Scale>(id)?;
                self.guarded(id, || scale.set_value(value))?;
            }
            Change::SliderRange(range) => {
                // Narrowing the range can clamp the value; that is not an edit.
                let scale = self.get::<gtk::Scale>(id)?;
                self.guarded(id, || {
                    scale.set_range(range.min, range.max);
                    scale.set_increments(range.step, range.step * 10.0);
                })?;
            }
            Change::Fraction(fraction) => self.get::<gtk::ProgressBar>(id)?.set_fraction(fraction),
            Change::Options(options) => {
                let dropdown = self.get::<gtk::DropDown>(id)?;
                self.guarded(id, || dropdown.set_model(Some(&string_list(options))))?;
            }
            Change::Selected(selected) => {
                let dropdown = self.get::<gtk::DropDown>(id)?;
                self.guarded(id, || dropdown.set_selected(list_position(selected)))?;
            }
            Change::Image(path) => self
                .get::<gtk::Picture>(id)?
                .set_paintable(Some(&texture(path)?)),
            Change::Order(order) => {
                let order = order
                    .iter()
                    .map(|child| Ok(self.native(*child)?.widget.clone()))
                    .collect::<Result<Vec<_>>>()?;
                self.host(id)?.reorder(&order);
            }
            Change::MinimumSize(size) => self.host(id)?.set_minimum(size),
            Change::Overlap(_) => {}
        }
        Ok(())
    }

    fn destroy(&mut self, id: WidgetId) {
        if let Some(native) = self.widgets.remove(&id) {
            for signal in native.signals.into_iter().chain(native.guarded) {
                native.widget.disconnect(signal);
            }
            if let Some(window) = native.widget.downcast_ref::<gtk::Window>() {
                window.destroy();
                self.main_loop.quit();
            } else if let Some(parent) = native.widget.parent().and_downcast::<Host>() {
                parent.remove(&native.widget);
            }
        }
    }

    fn set_frame(&mut self, id: WidgetId, rect: Rect) -> Result<()> {
        let widget = &self.native(id)?.widget;
        widget
            .parent()
            .and_downcast::<Host>()
            .ok_or("widget has no layout host")?
            .set_child_frame(widget, rect);
        Ok(())
    }

    fn measure(&mut self, id: WidgetId, constraints: Constraints) -> Result<Measurement> {
        let widget = &self.native(id)?.widget;
        let (min_width, width, _, _) = widget.measure(
            gtk::Orientation::Horizontal,
            constraints.height.map_or(-1, |h| h.round() as i32),
        );
        let (min_height, height, _, _) = widget.measure(
            gtk::Orientation::Vertical,
            constraints.width.map_or(-1, |w| w.round() as i32),
        );
        Ok(Measurement {
            minimum: Size {
                width: min_width as f32,
                height: min_height as f32,
            },
            preferred: Size {
                width: width as f32,
                height: height as f32,
            },
        })
    }

    fn post(&self, event: Event) {
        self.queue.push(event);
    }

    fn run_loop(
        mut dispatch: impl FnMut(&mut Self, &[Event]) -> Result<()> + 'static,
    ) -> Result<()> {
        gtk::init()?;
        let queue = Rc::new(Queue::default());
        let main_loop = glib::MainLoop::new(None, false);
        let mut backend = Self {
            widgets: HashMap::new(),
            queue: queue.clone(),
            main_loop: main_loop.clone(),
            settings_signals: Vec::new(),
        };
        if let Some(settings) = gtk::Settings::default() {
            for property in [
                "gtk-font-name",
                "gtk-theme-name",
                "gtk-application-prefer-dark-theme",
                "gtk-xft-dpi",
            ] {
                let queue = queue.clone();
                let signal = settings.connect_notify_local(Some(property), move |_, _| {
                    abort_on_panic(|| queue.push(Event::AppearanceChanged))
                });
                backend.settings_signals.push((settings.clone(), signal));
            }
        }
        abort_on_panic(|| dispatch(&mut backend, &[Event::Started]))?;
        let window = backend
            .widgets
            .values()
            .find_map(|native| native.widget.downcast_ref::<gtk::Window>().cloned())
            .ok_or("app did not create a window")?;
        let pair = Rc::new(RefCell::new((backend, dispatch)));
        let weak = Rc::downgrade(&pair);
        let failure = Rc::new(RefCell::new(None));
        let observed_failure = failure.clone();
        let exit_loop = main_loop.clone();
        *queue.dispatch.borrow_mut() = Some(Box::new(move |events| {
            if let Some(pair) = weak.upgrade() {
                let mut pair = pair.borrow_mut();
                let (backend, dispatch) = &mut *pair;
                if let Err(error) = dispatch(backend, &events) {
                    *observed_failure.borrow_mut() = Some(error);
                    exit_loop.quit();
                }
            }
        }));
        window.present();
        main_loop.run();
        queue.dispatch.borrow_mut().take();
        queue.pending.borrow_mut().clear();
        drop(pair);
        let result = match failure.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        };
        result
    }
}

impl Drop for GtkBackend {
    fn drop(&mut self) {
        for (settings, signal) in self.settings_signals.drain(..) {
            settings.disconnect(signal);
        }
        let windows: Vec<_> = self
            .widgets
            .values()
            .filter_map(|native| native.widget.downcast_ref::<gtk::Window>().cloned())
            .collect();
        for window in windows {
            window.destroy();
        }
        self.widgets.clear();
    }
}

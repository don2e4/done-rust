use std::borrow::Cow;
use std::path::Path;

pub type Error = Box<dyn std::error::Error>;
pub type Result<T> = std::result::Result<T, Error>;

/// Assigned by core, monotonically increasing, never reused during an app's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u64);

/// Logical units. Backends convert to device units and round.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

/// Logical rectangle relative to the parent widget's content origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A known dimension asks for the other one, e.g. the height for a given width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Constraints {
    pub width: Option<f32>,
    pub height: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measurement {
    pub minimum: Size,
    pub preferred: Size,
}

#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: Cow<'static, str>,
    pub size: Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Window,
    /// A native host for a row, column, or stack. Core places its children.
    Group,
    Text,
    Button,
    TextInput,
    Checkbox,
    Slider,
    Progress,
    Dropdown,
    /// A native viewport whose single child is laid out at the viewport width
    /// with unconstrained height. Its content host is backend-internal.
    Scroll,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Properties<'a> {
    Window(&'a WindowOptions),
    /// `overlap`: children overlap (a stack), so later ones must be drawn on top.
    Group {
        overlap: bool,
    },
    Text {
        value: &'a str,
        size: Option<f32>,
    },
    Button(&'a str),
    TextInput {
        value: &'a str,
        placeholder: &'a str,
        label: &'a str,
    },
    Checkbox {
        label: &'a str,
        checked: bool,
    },
    Slider {
        value: f64,
        range: SliderRange,
        label: &'a str,
    },
    Progress {
        fraction: f64,
        label: &'a str,
    },
    Dropdown {
        options: &'a [Cow<'static, str>],
        selected: Option<usize>,
        label: &'a str,
    },
    Scroll,
    /// Application-provided local file. Missing files and decode failures are errors.
    Image {
        path: &'a Path,
        label: &'a str,
    },
}

impl Properties<'_> {
    pub fn kind(self) -> Kind {
        match self {
            Self::Window(_) => Kind::Window,
            Self::Group { .. } => Kind::Group,
            Self::Text { .. } => Kind::Text,
            Self::Button(_) => Kind::Button,
            Self::TextInput { .. } => Kind::TextInput,
            Self::Checkbox { .. } => Kind::Checkbox,
            Self::Slider { .. } => Kind::Slider,
            Self::Progress { .. } => Kind::Progress,
            Self::Dropdown { .. } => Kind::Dropdown,
            Self::Scroll => Kind::Scroll,
            Self::Image { .. } => Kind::Image,
        }
    }
}

/// A property patch. Programmatic changes must not be reported back as events.
#[derive(Clone, Copy, Debug)]
pub enum Change<'a> {
    Text(&'a str),
    TextSize(Option<f32>),
    /// Visible label of a button or checkbox.
    Label(&'a str),
    /// Accessible name of a control without a visible label of its own.
    AccessibleLabel(&'a str),
    /// Text input contents.
    Value(&'a str),
    Placeholder(&'a str),
    Checked(bool),
    SliderValue(f64),
    SliderRange(SliderRange),
    Fraction(f64),
    Options(&'a [Cow<'static, str>]),
    Selected(Option<usize>),
    Image(&'a Path),
    /// The complete child order of a window, group, or scroll. Survivors are
    /// moved, never recreated; paint and keyboard order both follow it.
    Order(&'a [WidgetId]),
    /// Window only: the smallest content size the layout supports.
    MinimumSize(Size),
    /// Group only: whether its children now overlap.
    Overlap(bool),
}

impl Change<'_> {
    /// Whether the change can alter the widget's native measurement.
    pub fn affects_size(self) -> bool {
        matches!(
            self,
            Self::Text(_)
                | Self::TextSize(_)
                | Self::Label(_)
                | Self::Placeholder(_)
                | Self::Options(_)
                | Self::Selected(_)
                | Self::Image(_)
        )
    }
}

/// Native observations carry the value the control already shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Started,
    Click(WidgetId),
    /// Enter in a text input.
    Submit(WidgetId),
    Input(WidgetId, String),
    Toggle(WidgetId, bool),
    Slide(WidgetId, f64),
    Select(WidgetId, usize),
    Resized(Size),
    AppearanceChanged,
    Close,
}

impl Event {
    pub fn target(&self) -> Option<WidgetId> {
        match *self {
            Self::Click(id)
            | Self::Submit(id)
            | Self::Input(id, _)
            | Self::Toggle(id, _)
            | Self::Slide(id, _)
            | Self::Select(id, _) => Some(id),
            Self::Started | Self::Resized(_) | Self::AppearanceChanged | Self::Close => None,
        }
    }
}

/// Native objects are main-thread-only. Callbacks enqueue events rather than
/// synchronously calling the driver. Cross-thread posting is deferred until
/// the worker API is introduced.
pub trait Backend: Sized {
    fn create(
        &mut self,
        id: WidgetId,
        parent: Option<WidgetId>,
        properties: Properties<'_>,
    ) -> Result<()>;
    fn update(&mut self, id: WidgetId, change: Change<'_>) -> Result<()>;
    /// Called children-first. The widget's parent still exists.
    fn destroy(&mut self, id: WidgetId);
    fn set_frame(&mut self, id: WidgetId, rect: Rect) -> Result<()>;
    fn measure(&mut self, id: WidgetId, constraints: Constraints) -> Result<Measurement>;
    /// Width a vertical scroll bar takes from a scroll viewport's content, in
    /// logical units; zero where scroll bars overlay content.
    fn scrollbar_gutter(&self) -> f32 {
        0.0
    }
    fn post(&self, event: Event);
    fn run_loop(dispatch: impl FnMut(&mut Self, &[Event]) -> Result<()> + 'static) -> Result<()>;
}

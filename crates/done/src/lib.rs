//! Tiny declarative views backed by real native widgets.
#![forbid(unsafe_code)]

use done_core::{Backend, Driver, Size, WindowOptions};
use std::borrow::Cow;

pub use done_core::{Result, View};

pub mod prelude {
    pub use done_core::{
        button, checkbox, column, dropdown, image, keyed, progress, row, scroll, slider, stack,
        text, text_input, Align, Either, Modifiers, View,
    };
}

#[cfg(all(target_os = "linux", feature = "gtk4"))]
type NativeBackend = done_gtk4::GtkBackend;
#[cfg(all(windows, feature = "win32"))]
type NativeBackend = done_win32::Win32Backend;
#[cfg(all(target_os = "macos", feature = "appkit"))]
type NativeBackend = done_appkit::AppKitBackend;

#[cfg(not(any(
    all(target_os = "linux", feature = "gtk4"),
    all(windows, feature = "win32"),
    all(target_os = "macos", feature = "appkit"),
)))]
compile_error!(
    "done needs a native backend: Linux with `gtk4`, Windows with `win32`, or macOS with `appkit`"
);

pub struct App<S, F> {
    options: WindowOptions,
    state: S,
    view: F,
}

pub fn app<S, F, V>(title: impl Into<Cow<'static, str>>, state: S, view: F) -> App<S, F>
where
    F: Fn(&S) -> V,
    V: View<S>,
{
    App {
        options: WindowOptions {
            title: title.into(),
            size: Size {
                width: 320.0,
                height: 200.0,
            },
        },
        state,
        view,
    }
}

impl<S: 'static, F: 'static> App<S, F> {
    pub fn size(mut self, width: f32, height: f32) -> Self {
        assert!(
            width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
            "window size must be positive and finite"
        );
        self.options.size = Size { width, height };
        self
    }

    /// Run on the main thread. Report initialization/native failures to callers.
    pub fn run<V>(self) -> Result<()>
    where
        F: Fn(&S) -> V,
        V: View<S> + 'static,
        V::Retained: 'static,
    {
        let mut driver = Driver::new(self.options, self.state, self.view);
        NativeBackend::run_loop(move |backend, events| driver.dispatch(backend, events))
    }
}

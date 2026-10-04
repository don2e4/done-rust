//! Optional first-frame instrumentation. Absent from normal release builds.
use gtk4::{self as gtk, glib, prelude::*};
use std::cell::Cell;

pub fn install(window: &gtk::Window) {
    if std::env::var_os("DONE_PROBE").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    window.connect_map(|window| {
        if let Some(clock) = window.frame_clock() {
            let seen = Cell::new(false);
            let window = window.downgrade();
            clock.connect_after_paint(move |_| {
                crate::abort_on_panic(|| {
                    if !seen.replace(true) {
                        let timestamp = glib::monotonic_time();
                        if let Some(window) = window.upgrade() {
                            let renderer = window
                                .renderer()
                                .map(|renderer| renderer.type_().name().to_owned())
                                .unwrap_or_else(|| "unknown".into());
                            println!(
                                "DONE_FRAME {timestamp} {renderer} {} {}",
                                window.scale_factor(),
                                WidgetExt::display(&window).type_().name()
                            );
                        }
                    }
                });
            });
        }
    });
}

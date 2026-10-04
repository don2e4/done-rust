#![cfg(target_os = "linux")]

use done::prelude::*;
use gtk4::{self as gtk, glib, prelude::*};
use std::{cell::Cell, rc::Rc, time::Duration};

mod common;
use common::{border_box, descendants};

#[test]
#[ignore = "requires a desktop display and window manager; run with --ignored --test-threads=1"]
fn native_layout_follows_window_size_font_scale_and_theme_changes() {
    const WIDE: i32 = 500;
    let phase = Rc::new(Cell::new(0));
    let ticks_in_phase = Cell::new(0);
    let before = Cell::new(0);
    let observed_phase = phase.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let Some(window) = gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Window>().ok())
        else {
            return glib::ControlFlow::Continue;
        };
        if !window.is_mapped() || window.width() == 0 {
            return glib::ControlFlow::Continue;
        }
        let mut widgets = Vec::new();
        descendants(window.upcast_ref(), &mut widgets);
        let label = widgets
            .iter()
            .find_map(|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .filter(|l| l.text().starts_with("Clicked "))
                    .cloned()
            })
            .unwrap();
        let minus = widgets
            .iter()
            .find_map(|widget| widget.downcast_ref::<gtk::Button>().cloned())
            .unwrap();
        // Relayout is dispatched at idle priority, so give each step a bounded wait.
        let ticks = ticks_in_phase.get() + 1;
        ticks_in_phase.set(ticks);
        assert!(ticks <= 60, "phase {} did not settle", observed_phase.get());
        let centered = |width: i32| {
            let bounds = border_box(&label);
            (bounds.x() + bounds.width() / 2.0 - width as f32 / 2.0).abs() <= 1.0
        };
        let next = |phase: i32| {
            observed_phase.set(phase);
            ticks_in_phase.set(0);
        };
        match observed_phase.get() {
            0 => {
                assert!(centered(window.width()), "initial layout is centered");
                window.set_default_size(WIDE, 300);
                next(1);
            }
            1 => {
                if window.width() != WIDE || !centered(WIDE) {
                    return glib::ControlFlow::Continue;
                }
                before.set(minus.measure(gtk::Orientation::Vertical, -1).0);
                gtk::Settings::default()
                    .expect("default settings")
                    .set_gtk_font_name(Some("Sans 32"));
                next(2);
            }
            2 => {
                let (minimum, _, _, _) = minus.measure(gtk::Orientation::Vertical, -1);
                let bounds = border_box(&minus);
                if minimum <= before.get() || bounds.height() < minimum as f32 {
                    return glib::ControlFlow::Continue;
                }
                assert!(centered(WIDE), "layout stays centered after a font change");
                // Text scaling: 1.5 times the font resolution.
                before.set(minimum);
                let settings = gtk::Settings::default().expect("default settings");
                let dpi = settings.gtk_xft_dpi();
                settings.set_gtk_xft_dpi(if dpi > 0 { dpi } else { 96 * 1024 } * 3 / 2);
                next(3);
            }
            3 => {
                let (minimum, _, _, _) = minus.measure(gtk::Orientation::Vertical, -1);
                let bounds = border_box(&minus);
                if minimum <= before.get() || bounds.height() < minimum as f32 {
                    return glib::ControlFlow::Continue;
                }
                assert!(centered(WIDE), "layout stays centered after text scaling");
                gtk::Settings::default()
                    .expect("default settings")
                    .set_gtk_application_prefer_dark_theme(true);
                next(4);
            }
            4 => {
                // A theme change remeasures everything; the layout must still
                // honour every native minimum.
                if ticks < 4 {
                    return glib::ControlFlow::Continue;
                }
                for widget in [label.upcast_ref::<gtk::Widget>(), minus.upcast_ref()] {
                    let (width, _, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
                    let (height, _, _, _) = widget.measure(gtk::Orientation::Vertical, -1);
                    let bounds = border_box(widget);
                    assert!(bounds.width() >= width as f32 && bounds.height() >= height as f32);
                }
                assert!(centered(WIDE), "layout stays centered after a theme change");
                next(5);
                window.close();
                return glib::ControlFlow::Break;
            }
            _ => unreachable!(),
        }
        glib::ControlFlow::Continue
    });
    let watchdog = glib::timeout_add_local_once(Duration::from_secs(5), || {
        panic!("native layout check did not complete within five seconds");
    });
    done::app("Native layout check", 0, |count: &i32| {
        column((
            text(format!("Clicked {count} times")).size(24.0),
            row((button("−"), button("+"))).gap(8.0),
        ))
        .padding(16.0)
        .align(Align::Center)
    })
    .run()
    .unwrap();
    watchdog.remove();
    assert_eq!(phase.get(), 5);
}

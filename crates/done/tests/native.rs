#![cfg(target_os = "linux")]

use done::prelude::*;
use gtk4::{self as gtk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

mod common;
use common::descendants;

#[test]
#[ignore = "requires a desktop display; run with --ignored --test-threads=1"]
fn native_counter_batches_reuses_widgets_and_preserves_focus() {
    let phase = Rc::new(Cell::new(0));
    let saved = Rc::new(RefCell::new(None::<(gtk::Label, Vec<gtk::Button>)>));
    let builds = Rc::new(Cell::new(0));
    let observed_builds = builds.clone();
    let observed_phase = phase.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let Some(window) = gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Window>().ok())
        else {
            return glib::ControlFlow::Continue;
        };
        if !window.is_mapped() {
            return glib::ControlFlow::Continue;
        }
        let mut widgets = Vec::new();
        descendants(window.upcast_ref(), &mut widgets);
        let label = widgets
            .iter()
            .find_map(|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .filter(|label| label.text().starts_with("Clicked "))
                    .cloned()
            })
            .unwrap();
        let buttons: Vec<_> = widgets
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Button>().cloned())
            .collect();
        assert_eq!(buttons.len(), 2);
        match observed_phase.get() {
            0 => {
                assert_eq!(label.text(), "Clicked 0 times");
                assert_eq!(observed_builds.get(), 1);
                assert!(buttons[0].grab_focus());
                assert!(window.child_focus(gtk::DirectionType::TabForward));
                // is_focus: window-manager activation is outside this test's control.
                assert!(buttons[1].is_focus(), "Tab must follow source order");
                buttons[1].emit_clicked();
                buttons[1].emit_clicked();
                *saved.borrow_mut() = Some((label, buttons));
                observed_phase.set(1);
            }
            1 => {
                // The batch is dispatched at idle priority and may trail this timer
                // behind a slow first frame; wait for it and for a first allocation.
                if observed_builds.get() < 2 || window.width() == 0 {
                    return glib::ControlFlow::Continue;
                }
                assert_eq!(label.text(), "Clicked 2 times");
                assert_eq!(observed_builds.get(), 2, "native clicks share one rebuild");
                let saved = saved.borrow();
                let (old_label, old_buttons) = saved.as_ref().unwrap();
                assert_eq!(&label, old_label, "label identity survives the patch");
                assert_eq!(&buttons, old_buttons, "buttons survive the patch");
                assert!(buttons[1].is_focus(), "focus survives the patch");
                assert_eq!(
                    label.property::<gtk::AccessibleRole>("accessible-role"),
                    gtk::AccessibleRole::Label
                );
                for button in &buttons {
                    assert_eq!(
                        button.property::<gtk::AccessibleRole>("accessible-role"),
                        gtk::AccessibleRole::Button
                    );
                }
                for widget in widgets.iter().filter(|widget| {
                    widget.is::<gtk::Button>() || *widget == label.upcast_ref::<gtk::Widget>()
                }) {
                    // `width()`/`height()` report the content box, which excludes CSS
                    // padding and borders, while `measure` includes them.
                    let (minimum, _, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
                    let bounds = widget
                        .compute_bounds(widget)
                        .expect("allocated widget has bounds");
                    assert!(
                        bounds.width() >= minimum as f32 && bounds.height() > 0.0,
                        "native control has a valid allocation"
                    );
                }
                observed_phase.set(2);
                window.close();
                return glib::ControlFlow::Break;
            }
            _ => unreachable!(),
        }
        glib::ControlFlow::Continue
    });
    let watchdog = glib::timeout_add_local_once(Duration::from_secs(5), || {
        panic!("native counter did not complete within five seconds");
    });
    done::app("Native counter check", 0, move |count: &i32| {
        builds.set(builds.get() + 1);
        column((
            text(format!("Clicked {count} times")).size(24.0),
            row((
                button("−").on_click(|s: &mut i32| *s -= 1),
                button("+").on_click(|s: &mut i32| *s += 1),
            ))
            .gap(8.0),
        ))
        .padding(16.0)
        .align(Align::Center)
    })
    .run()
    .unwrap();
    watchdog.remove();
    assert_eq!(phase.get(), 2);
}

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

struct State {
    name: String,
    name_events: u32,
    checked: bool,
    volume: f64,
    max: f64,
    theme: usize,
    themes: Vec<&'static str>,
    /// Handler calls from the checkbox, slider, and dropdown.
    feedback: u32,
    items: Vec<u32>,
}

fn view(s: &State) -> impl View<State> {
    column((
        text_input(s.name.clone())
            .label("Name")
            .on_input(|s: &mut State, value| {
                s.name_events += 1;
                if !value.chars().any(|c| c.is_ascii_digit()) {
                    s.name = value.to_owned();
                }
            })
            .on_submit(|s: &mut State| s.items.push(100)),
        checkbox("Check", s.checked).on_toggle(|s: &mut State, checked| {
            s.feedback += 1;
            s.checked = checked;
        }),
        slider(s.volume, 0.0..=s.max)
            .step(1.0)
            .label("Volume")
            .on_change(|s: &mut State, volume| {
                s.feedback += 1;
                s.volume = volume;
            }),
        dropdown(s.themes.clone(), s.theme)
            .label("Theme")
            .on_select(|s: &mut State, theme| {
                s.feedback += 1;
                s.theme = theme;
            }),
        progress(s.volume / 100.0).label("Level"),
        button("program").on_click(|s: &mut State| {
            s.checked = false;
            s.volume = 30.0;
            s.max = 20.0;
            s.theme = 2;
            s.themes = vec!["A", "B", "C", "D"];
        }),
        button("rotate").on_click(|s: &mut State| s.items.rotate_left(1)),
        scroll(
            column(keyed(
                s.items
                    .iter()
                    .map(|item| (*item, checkbox(format!("item {item}"), false))),
            ))
            .align(Align::Stretch),
        )
        .grow(1.0),
    ))
    .gap(6.0)
    .padding(8.0)
    .align(Align::Stretch)
}

fn find<T: IsA<gtk::Widget>>(widgets: &[gtk::Widget], label: Option<&str>) -> T {
    widgets
        .iter()
        .filter_map(|widget| widget.downcast_ref::<T>())
        .find(|widget| {
            label.is_none_or(|label| {
                widget.property::<Option<String>>("label").as_deref() == Some(label)
            })
        })
        .cloned()
        .unwrap_or_else(|| panic!("no {} {label:?}", T::static_type().name()))
}

fn items(widgets: &[gtk::Widget]) -> Vec<(String, gtk::CheckButton)> {
    widgets
        .iter()
        .filter_map(|widget| widget.downcast_ref::<gtk::CheckButton>())
        .filter_map(|check| {
            let label = check.label()?.to_string();
            label.starts_with("item ").then(|| (label, check.clone()))
        })
        .collect()
}

fn viewport(row: &gtk::CheckButton) -> gtk::ScrolledWindow {
    row.ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast()
        .expect("rows are inside the scroll view")
}

#[test]
#[ignore = "requires a desktop display and window manager; run with --ignored --test-threads=1"]
fn native_widgets_echo_no_programmatic_changes_and_keep_identity() {
    let phase = Rc::new(Cell::new(0));
    let observed_phase = phase.clone();
    let ticks = Cell::new(0);
    let changed = Rc::new(Cell::new(0));
    let saved = RefCell::new(Vec::<(String, gtk::CheckButton)>::new());
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
        let tick = ticks.get() + 1;
        ticks.set(tick);
        assert!(tick <= 80, "phase {} did not settle", observed_phase.get());
        let mut widgets = Vec::new();
        descendants(window.upcast_ref(), &mut widgets);
        let entry: gtk::Entry = find(&widgets, None);
        let check: gtk::CheckButton = find(&widgets, Some("Check"));
        let scale: gtk::Scale = find(&widgets, None);
        let dropdown: gtk::DropDown = find(&widgets, None);
        let next = |phase: i32| {
            observed_phase.set(phase);
            ticks.set(0);
        };
        match observed_phase.get() {
            0 => {
                for (widget, role) in [
                    (
                        entry.upcast_ref::<gtk::Widget>(),
                        gtk::AccessibleRole::TextBox,
                    ),
                    (check.upcast_ref(), gtk::AccessibleRole::Checkbox),
                    (scale.upcast_ref(), gtk::AccessibleRole::Slider),
                    (dropdown.upcast_ref(), gtk::AccessibleRole::ComboBox),
                ] {
                    assert_eq!(widget.accessible_role(), role);
                }
                let counter = changed.clone();
                entry.connect_changed(move |_| counter.set(counter.get() + 1));
                // Typing goes through the entry's text delegate like a key press.
                let text = entry.delegate().expect("entry has a text delegate");
                entry.grab_focus();
                text.emit_by_name::<()>("insert-at-cursor", &[&"abc"]);
                entry.set_position(1);
                text.emit_by_name::<()>("insert-at-cursor", &[&"X"]);
                check.set_active(true);
                scale.set_value(80.0);
                dropdown.set_selected(1);
                next(1);
            }
            1 => {
                // Accepted edits: the control already shows the value, so no
                // setter may run and the caret stays after the typed character.
                if check.is_active() && entry.text() == "aXbc" && changed.get() == 2 {
                    assert_eq!(entry.position(), 2, "caret kept after an accepted edit");
                    let text = entry.delegate().unwrap();
                    text.emit_by_name::<()>("insert-at-cursor", &[&"7"]);
                    next(2);
                }
            }
            2 => {
                // A rejected edit is reverted once and does not echo back.
                if entry.text() == "aXbc" && changed.get() > 3 {
                    find::<gtk::Button>(&widgets, Some("program")).emit_clicked();
                    next(3);
                }
            }
            3 => {
                let model = dropdown.model().expect("dropdown has a model");
                if model.n_items() == 4 {
                    assert!(!check.is_active());
                    assert_eq!(scale.value(), 20.0, "value clamped to the narrowed range");
                    assert_eq!(dropdown.selected(), 2);
                    assert_eq!(scale.adjustment().upper(), 20.0);
                    let items = items(&widgets);
                    let first = &items[0].1;
                    assert!(first.grab_focus());
                    *saved.borrow_mut() = items;
                    find::<gtk::Button>(&widgets, Some("rotate")).emit_clicked();
                    next(4);
                }
            }
            4 => {
                let now = items(&widgets);
                let before = saved.borrow();
                if now[0].0 != before[0].0 {
                    let rotated: Vec<_> = before[1..].iter().chain(&before[..1]).cloned().collect();
                    assert_eq!(now, rotated, "rows move; their native widgets survive");
                    assert!(before[0].1.is_focus(), "the moved row keeps focus");
                    entry.emit_activate();
                    next(5);
                }
            }
            5 => {
                let now = items(&widgets);
                if now.len() == 31 {
                    let before = saved.borrow();
                    assert_eq!(now.last().unwrap().0, "item 100");
                    assert!(
                        before
                            .iter()
                            .all(|(_, old)| now.iter().any(|(_, new)| new == old)),
                        "inserting a key leaves the other rows alone"
                    );
                    // The row's own viewport: the dropdown's popup has another.
                    let scrolled = viewport(&now[0].1);
                    let adjustment = scrolled.vadjustment();
                    assert!(
                        adjustment.upper() > adjustment.page_size(),
                        "content taller than the viewport scrolls"
                    );
                    assert!(now.last().unwrap().1.grab_focus());
                    next(6);
                }
            }
            6 => {
                let scrolled = viewport(&items(&widgets)[0].1);
                if scrolled.vadjustment().value() > 0.0 {
                    next(7);
                    window.close();
                    return glib::ControlFlow::Break;
                }
            }
            _ => unreachable!(),
        }
        glib::ControlFlow::Continue
    });
    let watchdog = glib::timeout_add_local_once(Duration::from_secs(10), || {
        panic!("native widget check did not complete within ten seconds");
    });
    let state = State {
        name: String::new(),
        name_events: 0,
        checked: false,
        volume: 50.0,
        max: 100.0,
        theme: 0,
        themes: vec!["System", "Light", "Dark"],
        feedback: 0,
        items: (1..=30).collect(),
    };
    let report = Rc::new(Cell::new((0, 0)));
    let observed = report.clone();
    done::app("Native widget check", state, move |s: &State| {
        observed.set((s.name_events, s.feedback));
        view(s)
    })
    .size(320.0, 480.0)
    .run()
    .unwrap();
    watchdog.remove();
    assert_eq!(phase.get(), 7);
    let (name_events, feedback) = report.get();
    assert_eq!(
        name_events, 3,
        "two accepted and one rejected edit; programmatic changes never echo"
    );
    assert_eq!(
        feedback, 3,
        "three user edits; the programmatic changes caused no handler calls"
    );
}

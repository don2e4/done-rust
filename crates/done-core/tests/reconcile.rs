use done_core::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

/// Records every native operation; leaves measure 40x20 unless configured.
#[derive(Default)]
struct Mock {
    operations: Vec<String>,
    fail_update: bool,
    sizes: HashMap<u64, Measurement>,
    /// Leaves whose height doubles when narrower than their preferred width.
    wrapping: HashSet<u64>,
    missing: Option<PathBuf>,
    gutter: f32,
}

fn size(width: f32, height: f32) -> Measurement {
    let size = Size { width, height };
    Measurement {
        minimum: size,
        preferred: size,
    }
}

impl Mock {
    fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.operations)
    }
}

impl Backend for Mock {
    fn create(
        &mut self,
        id: WidgetId,
        parent: Option<WidgetId>,
        props: Properties<'_>,
    ) -> Result<()> {
        if let Properties::Image { path, .. } = props {
            if self.missing.as_deref() == Some(path) {
                return Err(format!("{}: no such file", path.display()).into());
            }
        }
        self.operations.push(format!(
            "create {} {:?} {:?}",
            id.0,
            parent.map(|p| p.0),
            props.kind()
        ));
        Ok(())
    }

    fn update(&mut self, id: WidgetId, change: Change<'_>) -> Result<()> {
        if self.fail_update {
            return Err("simulated native failure".into());
        }
        self.operations.push(format!("update {} {change:?}", id.0));
        Ok(())
    }

    fn destroy(&mut self, id: WidgetId) {
        self.operations.push(format!("destroy {}", id.0));
    }

    fn set_frame(&mut self, id: WidgetId, rect: Rect) -> Result<()> {
        self.operations.push(format!(
            "frame {} {},{} {}x{}",
            id.0, rect.x, rect.y, rect.width, rect.height
        ));
        Ok(())
    }

    fn measure(&mut self, id: WidgetId, constraints: Constraints) -> Result<Measurement> {
        self.operations.push(match constraints {
            Constraints {
                width: None,
                height: None,
            } => format!("measure {}", id.0),
            Constraints { width, height } => format!("measure {} {width:?}x{height:?}", id.0),
        });
        let mut measurement = self.sizes.get(&id.0).copied().unwrap_or(size(40.0, 20.0));
        if self.wrapping.contains(&id.0) {
            measurement.minimum.width = measurement.preferred.width / 2.0;
            if constraints
                .width
                .is_some_and(|width| width < measurement.preferred.width)
            {
                measurement.minimum.height *= 2.0;
                measurement.preferred.height *= 2.0;
            }
        }
        Ok(measurement)
    }

    fn scrollbar_gutter(&self) -> f32 {
        self.gutter
    }

    fn post(&self, _: Event) {}

    fn run_loop(_: impl FnMut(&mut Self, &[Event]) -> Result<()> + 'static) -> Result<()> {
        unreachable!()
    }
}

fn options(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        title: "Test".into(),
        size: Size { width, height },
    }
}

fn frames(operations: &[String]) -> Vec<&str> {
    operations
        .iter()
        .filter(|op| op.starts_with("frame"))
        .map(String::as_str)
        .collect()
}

/// State replaced step by step by clicking widget 3, the first view built.
struct Scripted<T> {
    value: T,
    script: VecDeque<T>,
}

fn scripted<T>(value: T, script: impl IntoIterator<Item = T>) -> Scripted<T> {
    Scripted {
        value,
        script: script.into_iter().collect(),
    }
}

fn advance<T>(state: &mut Scripted<T>) {
    if let Some(value) = state.script.pop_front() {
        state.value = value;
    }
}

const NEXT: Event = Event::Click(WidgetId(3));

fn counter(count: &i32) -> impl View<i32> {
    column((
        text(format!("Count {count}")),
        row((
            button("-").on_click(|s: &mut i32| *s -= 1),
            button("+").on_click(|s: &mut i32| *s += 1),
        ))
        .gap(8.0),
    ))
    .padding(16.0)
    .align(Align::Center)
}

#[test]
fn counter_batch_patches_one_property_and_reuses_measurements() {
    let mut mock = Mock::default();
    let builds = Rc::new(Cell::new(0));
    let observed = builds.clone();
    let mut driver = Driver::new(options(200.0, 100.0), 0, move |s: &i32| {
        observed.set(observed.get() + 1);
        counter(s)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    let operations = mock.take();
    assert_eq!(
        operations
            .iter()
            .filter(|op| op.starts_with("create"))
            .count(),
        6
    );
    assert_eq!(
        frames(&operations),
        [
            "frame 2 0,0 200x100",
            "frame 3 80,16 40x20",
            "frame 4 56,36 88x20",
            "frame 5 0,0 40x20",
            "frame 6 48,0 40x20",
        ]
    );
    assert!(
        operations.contains(&"update 1 MinimumSize(Size { width: 120.0, height: 72.0 })".into())
    );
    driver
        .dispatch(
            &mut mock,
            &[Event::Click(WidgetId(6)), Event::Click(WidgetId(6))],
        )
        .unwrap();
    assert_eq!(*driver.state(), 2);
    assert_eq!(builds.get(), 2, "one rebuild per batch");
    assert_eq!(mock.take(), ["update 3 Text(\"Count 2\")", "measure 3"]);
    driver
        .dispatch(
            &mut mock,
            &[Event::Click(WidgetId(5)), Event::Click(WidgetId(6))],
        )
        .unwrap();
    assert!(
        mock.operations.is_empty(),
        "identical description must emit no native operations"
    );
}

#[test]
fn refreshing_captured_handlers_does_not_recreate_controls() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(200.0, 100.0), 1, |s: &i32| {
        let increment = *s;
        button("Add current").on_click(move |s: &mut i32| *s += increment)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    mock.take();
    driver
        .dispatch(&mut mock, &[Event::Click(WidgetId(2))])
        .unwrap();
    driver
        .dispatch(&mut mock, &[Event::Click(WidgetId(2))])
        .unwrap();
    assert_eq!(
        *driver.state(),
        4,
        "latest capture should be used on the next tick"
    );
    assert!(mock.operations.is_empty());
}

#[test]
fn resize_repositions_without_remeasuring_and_theme_change_remeasures() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(200.0, 100.0), 0, counter);
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    mock.take();
    driver
        .dispatch(
            &mut mock,
            &[Event::Resized(Size {
                width: 300.0,
                height: 100.0,
            })],
        )
        .unwrap();
    assert_eq!(
        mock.take(),
        [
            "frame 2 0,0 300x100",
            "frame 3 130,16 40x20",
            "frame 4 106,36 88x20"
        ],
        "only the root and the re-centred children move"
    );
    driver
        .dispatch(&mut mock, &[Event::AppearanceChanged])
        .unwrap();
    assert_eq!(mock.take(), ["measure 3", "measure 5", "measure 6"]);
}

#[test]
fn shutdown_and_patch_failure_destroy_children_before_parents() {
    for fail in [false, true] {
        let mut mock = Mock::default();
        let mut driver = Driver::new(options(200.0, 100.0), 0, counter);
        driver.dispatch(&mut mock, &[Event::Started]).unwrap();
        mock.take();
        mock.fail_update = fail;
        let event = if fail {
            Event::Click(WidgetId(6))
        } else {
            Event::Close
        };
        assert_eq!(driver.dispatch(&mut mock, &[event]).is_err(), fail);
        assert_eq!(
            mock.take(),
            [
                "destroy 3",
                "destroy 5",
                "destroy 6",
                "destroy 4",
                "destroy 2",
                "destroy 1"
            ]
        );
        driver
            .dispatch(&mut mock, &[Event::Click(WidgetId(6)), Event::Close])
            .unwrap();
        assert!(
            mock.operations.is_empty(),
            "no live handlers after teardown"
        );
    }
}

#[test]
fn font_button_label_and_gap_changes_patch_only_affected_widgets() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(200.0, 100.0), 0, |s: &i32| {
        column((
            text("Fixed text").size(24.0 + *s as f32),
            button(format!("Value {s}")).on_click(|s: &mut i32| *s += 1),
        ))
        .gap(*s as f32 * 8.0)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    mock.take();
    driver
        .dispatch(&mut mock, &[Event::Click(WidgetId(4))])
        .unwrap();
    assert_eq!(
        mock.take(),
        [
            "update 3 TextSize(Some(25.0))",
            "update 4 Label(\"Value 1\")",
            "measure 3",
            "measure 4",
            "update 1 MinimumSize(Size { width: 40.0, height: 48.0 })",
            "frame 4 0,28 40x20",
        ]
    );
}

fn keyed_list(state: &Scripted<Vec<u32>>) -> impl View<Scripted<Vec<u32>>> {
    column((
        button("next").on_click(advance::<Vec<u32>>),
        column(keyed(
            state
                .value
                .iter()
                .map(|item| (*item, text(format!("item {item}")))),
        )),
    ))
}

#[test]
fn keyed_reorder_moves_survivors_and_new_keys_get_new_identities() {
    let mut mock = Mock::default();
    let state = scripted(vec![1, 2, 3], [vec![3, 1, 2], vec![3, 9, 2], vec![3, 9, 2]]);
    let mut driver = Driver::new(options(200.0, 200.0), state, keyed_list);
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    // 1 window, 2 column, 3 button, 4 list, 5..=7 items 1, 2, 3.
    mock.take();

    driver.dispatch(&mut mock, &[NEXT]).unwrap();
    let operations = mock.take();
    assert_eq!(
        operations[0], "update 4 Order([WidgetId(7), WidgetId(5), WidgetId(6)])",
        "survivors move natively"
    );
    assert!(
        !operations
            .iter()
            .any(|op| op.starts_with("create") || op.starts_with("destroy")),
        "a reorder keeps every native control: {operations:?}"
    );
    assert_eq!(
        frames(&operations),
        [
            "frame 7 0,0 40x20",
            "frame 5 0,20 40x20",
            "frame 6 0,40 40x20"
        ]
    );

    driver.dispatch(&mut mock, &[NEXT]).unwrap();
    let operations = mock.take();
    assert_eq!(
        operations[..3],
        [
            "create 8 Some(4) Text",
            "destroy 5",
            "update 4 Order([WidgetId(7), WidgetId(8), WidgetId(6)])"
        ],
        "key 9 is new; key 1's widget is destroyed, never reused"
    );

    driver.dispatch(&mut mock, &[NEXT]).unwrap();
    assert!(
        mock.take().is_empty(),
        "same keys and values: no native work"
    );
}

#[test]
fn duplicate_keys_fail_clearly_and_tear_down() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(200.0, 100.0), (), |_: &()| {
        column(keyed([(1, text("a")), (1, text("b"))]))
    });
    let error = driver.dispatch(&mut mock, &[Event::Started]).unwrap_err();
    assert_eq!(error.to_string(), "duplicate key 1 among keyed siblings");
    let operations = mock.take();
    assert_eq!(
        operations[operations.len() - 2..],
        ["destroy 2", "destroy 1"]
    );
}

struct Form {
    name: String,
    agree: bool,
    volume: f64,
    theme: usize,
}

fn form(state: &Form) -> impl View<Form> {
    column((
        text_input(state.name.clone())
            .label("Name")
            .on_input(|s: &mut Form, value| {
                if value.len() <= 5 {
                    s.name = value.to_owned();
                }
            }),
        checkbox("Agree", state.agree).on_toggle(|s: &mut Form, checked| s.agree = checked),
        slider(state.volume, 0.0..=10.0)
            .step(1.0)
            .on_change(|s: &mut Form, value| s.volume = value.round()),
        dropdown(["System", "Light", "Dark"], state.theme)
            .on_select(|s: &mut Form, index| s.theme = index),
        text_input("fixed"),
    ))
}

#[test]
fn accepted_native_edits_emit_no_setter_and_rejected_edits_revert() {
    let mut mock = Mock::default();
    let state = Form {
        name: String::new(),
        agree: false,
        volume: 3.0,
        theme: 0,
    };
    let mut driver = Driver::new(options(300.0, 300.0), state, form);
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    // 1 window, 2 column, 3 name, 4 agree, 5 volume, 6 theme, 7 fixed.
    mock.take();
    driver
        .dispatch(
            &mut mock,
            &[
                Event::Input(WidgetId(3), "Ada".into()),
                Event::Toggle(WidgetId(4), true),
                Event::Slide(WidgetId(5), 7.0),
                Event::Select(WidgetId(6), 2),
            ],
        )
        .unwrap();
    let state = driver.state();
    assert_eq!(
        (state.name.as_str(), state.agree, state.volume, state.theme),
        ("Ada", true, 7.0, 2)
    );
    assert!(
        mock.take().is_empty(),
        "values the control already shows need no setter"
    );

    driver
        .dispatch(
            &mut mock,
            &[
                Event::Input(WidgetId(3), "Ada Lovelace".into()),
                Event::Slide(WidgetId(5), 4.4),
                Event::Input(WidgetId(7), "edited".into()),
            ],
        )
        .unwrap();
    assert_eq!(driver.state().name, "Ada", "the state rejected the edit");
    assert_eq!(
        mock.take(),
        [
            "update 3 Value(\"Ada\")",
            "update 5 SliderValue(4.0)",
            "update 7 Value(\"fixed\")"
        ],
        "rejected, adjusted, and handler-less edits snap back to the description"
    );
}

#[test]
fn switching_kinds_replaces_the_subtree_and_ignores_stale_events() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(200.0, 100.0), true, |flag: &bool| {
        column((
            if *flag {
                Either::Left(row((button("A").on_click(|s: &mut bool| *s = false),)))
            } else {
                Either::Right(text("B"))
            },
            text("end"),
        ))
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    // 1 window, 2 column, 3 row, 4 button A, 5 end.
    mock.take();
    driver
        .dispatch(&mut mock, &[Event::Click(WidgetId(4))])
        .unwrap();
    assert!(!*driver.state());
    let operations = mock.take();
    assert_eq!(
        operations[..4],
        [
            "destroy 4",
            "destroy 3",
            "create 6 Some(2) Text",
            "update 2 Order([WidgetId(6), WidgetId(5)])"
        ],
        "the replaced subtree is destroyed children first; its successor takes its place"
    );
    driver
        .dispatch(&mut mock, &[Event::Click(WidgetId(4))])
        .unwrap();
    assert!(
        mock.take().is_empty(),
        "a stale event for a destroyed widget is ignored"
    );
}

#[test]
fn optional_children_are_inserted_in_source_order() {
    let mut mock = Mock::default();
    let state = scripted(false, [true, false]);
    let mut driver = Driver::new(options(200.0, 200.0), state, |s: &Scripted<bool>| {
        column((
            button("toggle").on_click(advance::<bool>),
            text("a"),
            s.value.then(|| text("b")),
            text("c"),
        ))
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    // 1 window, 2 column, 3 toggle, 4 a, 5 c.
    mock.take();
    driver.dispatch(&mut mock, &[NEXT]).unwrap();
    let operations = mock.take();
    assert_eq!(
        operations[..2],
        [
            "create 6 Some(2) Text",
            "update 2 Order([WidgetId(3), WidgetId(4), WidgetId(6), WidgetId(5)])"
        ]
    );
    assert!(operations.contains(&"frame 5 0,60 40x20".into()));
    driver.dispatch(&mut mock, &[NEXT]).unwrap();
    assert_eq!(mock.take()[0], "destroy 6");
}

#[test]
fn stack_sizes_to_its_largest_child_and_places_each_by_alignment() {
    for (align, small, large) in [
        (Align::Center, "frame 4 40,30 40x20", "frame 5 10,10 100x60"),
        (Align::End, "frame 4 70,50 40x20", "frame 5 10,10 100x60"),
        (
            Align::Stretch,
            "frame 4 10,10 100x60",
            "frame 5 10,10 100x60",
        ),
    ] {
        let mut mock = Mock::default();
        mock.sizes.insert(5, size(100.0, 60.0));
        let mut driver = Driver::new(options(300.0, 300.0), (), move |_: &()| {
            column((stack((text("small"), text("large")))
                .padding(10.0)
                .align(align),))
        });
        driver.dispatch(&mut mock, &[Event::Started]).unwrap();
        let operations = mock.take();
        assert_eq!(
            frames(&operations)[1..],
            ["frame 3 0,0 120x80", small, large],
            "{align:?}"
        );
    }
}

#[test]
fn grow_shares_spare_space_and_stretch_passes_width_constraints() {
    let mut mock = Mock::default();
    mock.wrapping.insert(6);
    mock.sizes.insert(6, size(80.0, 20.0));
    let mut driver = Driver::new(options(200.0, 100.0), (), |_: &()| {
        column((
            row((text("grows").grow(1.0), text("fixed"))),
            text("wrapping text"),
        ))
        .align(Align::Stretch)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    let operations = mock.take();
    assert_eq!(
        frames(&operations),
        [
            "frame 2 0,0 200x100",
            "frame 3 0,0 200x20",
            "frame 4 0,0 160x20",
            "frame 5 160,0 40x20",
            "frame 6 0,20 200x20",
        ]
    );
    driver
        .dispatch(
            &mut mock,
            &[Event::Resized(Size {
                width: 60.0,
                height: 100.0,
            })],
        )
        .unwrap();
    let operations = mock.take();
    assert!(
        operations.contains(&"measure 6 Some(60.0)xNone".into()),
        "the stretched leaf is asked for its height at the new width: {operations:?}"
    );
    assert!(operations.contains(&"frame 6 0,20 60x40".into()));
}

#[test]
fn scroll_content_uses_viewport_width_and_does_not_set_window_minimum() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(200.0, 100.0), (), |_: &()| {
        column((
            text("title"),
            scroll(column(keyed((0..10).map(|i| (i, text(format!("{i}"))))))).grow(1.0),
        ))
        .align(Align::Stretch)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    let operations = mock.take();
    let placed = frames(&operations);
    assert_eq!(
        placed[..4],
        [
            "frame 2 0,0 200x100",
            "frame 3 0,0 200x20",
            "frame 4 0,20 200x80",
            "frame 5 0,0 200x200"
        ],
        "viewport fills the rest; content is as tall as its ten rows"
    );
    assert!(operations.contains(&"update 1 MinimumSize(Size { width: 40.0, height: 20.0 })".into()));

    // A scroll bar that takes space narrows the content, not the viewport.
    let mut mock = Mock {
        gutter: 16.0,
        ..Mock::default()
    };
    let mut driver = Driver::new(options(200.0, 100.0), (), |_: &()| {
        column((scroll(column((text("row"),)).align(Align::Stretch)).grow(1.0),))
            .align(Align::Stretch)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    let operations = mock.take();
    assert_eq!(
        frames(&operations)[1..4],
        [
            "frame 3 0,0 200x100",
            "frame 4 0,0 184x20",
            "frame 5 0,0 184x20"
        ]
    );
}

#[test]
fn image_load_failure_is_returned_and_tears_down() {
    let mut mock = Mock {
        missing: Some(PathBuf::from("missing.png")),
        ..Mock::default()
    };
    let mut driver = Driver::new(options(200.0, 100.0), (), |_: &()| {
        column((text("before"), image("missing.png").label("Logo")))
    });
    let error = driver.dispatch(&mut mock, &[Event::Started]).unwrap_err();
    assert_eq!(error.to_string(), "missing.png: no such file");
    assert_eq!(mock.take()[3..], ["destroy 3", "destroy 2", "destroy 1"]);
}

#[test]
fn unchanged_full_form_emits_no_native_operations() {
    let mut mock = Mock::default();
    let mut driver = Driver::new(options(400.0, 400.0), 0, |count: &i32| {
        column((
            button("tick").on_click(|s: &mut i32| *s += 1),
            text("Settings").size(20.0),
            row((
                text_input("name")
                    .placeholder("Name")
                    .label("Name")
                    .grow(1.0),
                checkbox("On", true),
            ))
            .gap(4.0),
            slider(5.0, 0.0..=10.0).label("Level"),
            progress(0.5).label("Done"),
            dropdown(["a", "b"], 1).label("Choice"),
            scroll(column((text("x"), text("y")))),
            stack((text("under"), text("over"))).align(Align::Center),
            (*count > 100).then(|| text("never")),
        ))
        .gap(8.0)
        .padding(8.0)
    });
    driver.dispatch(&mut mock, &[Event::Started]).unwrap();
    mock.take();
    driver
        .dispatch(&mut mock, &[Event::Click(WidgetId(3))])
        .unwrap();
    assert_eq!(*driver.state(), 1);
    assert!(mock.take().is_empty());
}

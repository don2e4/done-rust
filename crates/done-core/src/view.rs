use crate::runtime::{BuildContext, EventHandler, Value};
use crate::{Backend, Change, Event, Properties, Result, SliderRange, WidgetId};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;
use std::ops::RangeInclusive;
use std::path::PathBuf;

/// A cheap, typed description. Only the retained state survives rebuilding.
pub trait View<State> {
    type Retained;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, State, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained>;

    fn rebuild<B: Backend>(
        self,
        retained: &mut Self::Retained,
        cx: &mut BuildContext<'_, State, B>,
        parent: WidgetId,
    ) -> Result<()>;

    /// The native widgets this view placed directly in its parent, in order.
    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>);
}

fn remove_view<S, V: View<S>, B: Backend>(retained: &V::Retained, cx: &mut BuildContext<'_, S, B>) {
    let mut roots = Vec::new();
    V::roots(retained, &mut roots);
    for root in roots {
        cx.remove(root);
    }
}

/// No callback installed.
pub struct NoHandler;

/// An installed callback.
pub struct Handler<F>(F);

/// A callback receiving the state and the value the control reported.
pub trait Callback<S, A> {
    const ACTIVE: bool;
    fn call(&mut self, state: &mut S, argument: A);
}

impl<S, A> Callback<S, A> for NoHandler {
    const ACTIVE: bool = false;
    fn call(&mut self, _: &mut S, _: A) {}
}

impl<S, F: FnMut(&mut S)> Callback<S, ()> for Handler<F> {
    const ACTIVE: bool = true;
    fn call(&mut self, state: &mut S, _: ()) {
        (self.0)(state)
    }
}

impl<'a, S, F: FnMut(&mut S, &str)> Callback<S, &'a str> for Handler<F> {
    const ACTIVE: bool = true;
    fn call(&mut self, state: &mut S, value: &'a str) {
        (self.0)(state, value)
    }
}

impl<S, F: FnMut(&mut S, bool)> Callback<S, bool> for Handler<F> {
    const ACTIVE: bool = true;
    fn call(&mut self, state: &mut S, value: bool) {
        (self.0)(state, value)
    }
}

impl<S, F: FnMut(&mut S, f64)> Callback<S, f64> for Handler<F> {
    const ACTIVE: bool = true;
    fn call(&mut self, state: &mut S, value: f64) {
        (self.0)(state, value)
    }
}

impl<S, F: FnMut(&mut S, usize)> Callback<S, usize> for Handler<F> {
    const ACTIVE: bool = true;
    fn call(&mut self, state: &mut S, value: usize) {
        (self.0)(state, value)
    }
}

/// Install `handler` when any of its callbacks is active; otherwise make sure
/// a callback from the previous description no longer runs.
fn install<S, B: Backend>(
    cx: &mut BuildContext<'_, S, B>,
    id: WidgetId,
    active: bool,
    handler: impl EventHandler<S> + 'static,
) {
    if active {
        cx.set_handler(id, handler);
    } else {
        cx.clear_handler(id);
    }
}

pub struct Text {
    value: Cow<'static, str>,
    size: Option<f32>,
}

pub fn text(value: impl Into<Cow<'static, str>>) -> Text {
    Text {
        value: value.into(),
        size: None,
    }
}

impl Text {
    pub fn size(mut self, size: f32) -> Self {
        assert!(
            size.is_finite() && size > 0.0,
            "text size must be positive and finite"
        );
        self.size = Some(size);
        self
    }
}

pub struct TextState {
    id: WidgetId,
    value: Cow<'static, str>,
    size: Option<f32>,
}

impl<S> View<S> for Text {
    type Retained = TextState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<TextState> {
        let id = cx.create(
            parent,
            Properties::Text {
                value: &self.value,
                size: self.size,
            },
        )?;
        Ok(TextState {
            id,
            value: self.value,
            size: self.size,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut TextState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if self.value != old.value {
            cx.update(old.id, Change::Text(&self.value))?;
            old.value = self.value;
        }
        if self.size != old.size {
            cx.update(old.id, Change::TextSize(self.size))?;
            old.size = self.size;
        }
        Ok(())
    }

    fn roots(retained: &TextState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

pub struct Button<H = NoHandler> {
    label: Cow<'static, str>,
    handler: H,
}

pub fn button(label: impl Into<Cow<'static, str>>) -> Button {
    Button {
        label: label.into(),
        handler: NoHandler,
    }
}

impl<H> Button<H> {
    pub fn on_click<S, F: FnMut(&mut S) + 'static>(self, handler: F) -> Button<Handler<F>> {
        Button {
            label: self.label,
            handler: Handler(handler),
        }
    }
}

pub struct ButtonState {
    id: WidgetId,
    label: Cow<'static, str>,
}

struct OnClick<H>(H);

impl<S, H: Callback<S, ()>> EventHandler<S> for OnClick<H> {
    fn handle(&mut self, state: &mut S, event: &Event) {
        if let Event::Click(_) = event {
            self.0.call(state, ());
        }
    }
}

impl<S, H: Callback<S, ()> + 'static> View<S> for Button<H> {
    type Retained = ButtonState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<ButtonState> {
        let id = cx.create(parent, Properties::Button(&self.label))?;
        install(cx, id, H::ACTIVE, OnClick(self.handler));
        Ok(ButtonState {
            id,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut ButtonState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if self.label != old.label {
            cx.update(old.id, Change::Label(&self.label))?;
            old.label = self.label;
        }
        install(cx, old.id, H::ACTIVE, OnClick(self.handler));
        Ok(())
    }

    fn roots(retained: &ButtonState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

/// A single-line, controlled text input. An edit the state does not accept is
/// reverted to the described value.
pub struct TextInput<I = NoHandler, U = NoHandler> {
    value: Cow<'static, str>,
    placeholder: Cow<'static, str>,
    label: Cow<'static, str>,
    input: I,
    submit: U,
}

pub fn text_input(value: impl Into<Cow<'static, str>>) -> TextInput {
    TextInput {
        value: value.into(),
        placeholder: Cow::Borrowed(""),
        label: Cow::Borrowed(""),
        input: NoHandler,
        submit: NoHandler,
    }
}

impl<I, U> TextInput<I, U> {
    pub fn placeholder(mut self, placeholder: impl Into<Cow<'static, str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The accessible name; it is not drawn.
    pub fn label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn on_input<S, F: FnMut(&mut S, &str) + 'static>(
        self,
        handler: F,
    ) -> TextInput<Handler<F>, U> {
        TextInput {
            value: self.value,
            placeholder: self.placeholder,
            label: self.label,
            input: Handler(handler),
            submit: self.submit,
        }
    }

    /// Enter pressed in the input.
    pub fn on_submit<S, F: FnMut(&mut S) + 'static>(self, handler: F) -> TextInput<I, Handler<F>> {
        TextInput {
            value: self.value,
            placeholder: self.placeholder,
            label: self.label,
            input: self.input,
            submit: Handler(handler),
        }
    }
}

pub struct TextInputState {
    id: WidgetId,
    placeholder: Cow<'static, str>,
    label: Cow<'static, str>,
}

struct OnText<I, U>(I, U);

impl<S, I: for<'a> Callback<S, &'a str>, U: Callback<S, ()>> EventHandler<S> for OnText<I, U> {
    fn handle(&mut self, state: &mut S, event: &Event) {
        match event {
            Event::Input(_, value) => self.0.call(state, value),
            Event::Submit(_) => self.1.call(state, ()),
            _ => {}
        }
    }
}

impl<S, I, U> View<S> for TextInput<I, U>
where
    I: for<'a> Callback<S, &'a str> + 'static,
    U: Callback<S, ()> + 'static,
{
    type Retained = TextInputState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<TextInputState> {
        let id = cx.create(
            parent,
            Properties::TextInput {
                value: &self.value,
                placeholder: &self.placeholder,
                label: &self.label,
            },
        )?;
        cx.set_value(id, Value::Text(self.value.into_owned()));
        let active = <I as Callback<S, &str>>::ACTIVE || U::ACTIVE;
        install(cx, id, active, OnText(self.input, self.submit));
        Ok(TextInputState {
            id,
            placeholder: self.placeholder,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut TextInputState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if !matches!(cx.value(old.id), Value::Text(shown) if *shown == *self.value) {
            cx.update(old.id, Change::Value(&self.value))?;
            cx.set_value(old.id, Value::Text(self.value.into_owned()));
        }
        if self.placeholder != old.placeholder {
            cx.update(old.id, Change::Placeholder(&self.placeholder))?;
            old.placeholder = self.placeholder;
        }
        if self.label != old.label {
            cx.update(old.id, Change::AccessibleLabel(&self.label))?;
            old.label = self.label;
        }
        let active = <I as Callback<S, &str>>::ACTIVE || U::ACTIVE;
        install(cx, old.id, active, OnText(self.input, self.submit));
        Ok(())
    }

    fn roots(retained: &TextInputState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

pub struct Checkbox<H = NoHandler> {
    label: Cow<'static, str>,
    checked: bool,
    handler: H,
}

pub fn checkbox(label: impl Into<Cow<'static, str>>, checked: bool) -> Checkbox {
    Checkbox {
        label: label.into(),
        checked,
        handler: NoHandler,
    }
}

impl<H> Checkbox<H> {
    pub fn on_toggle<S, F: FnMut(&mut S, bool) + 'static>(
        self,
        handler: F,
    ) -> Checkbox<Handler<F>> {
        Checkbox {
            label: self.label,
            checked: self.checked,
            handler: Handler(handler),
        }
    }
}

pub struct CheckboxState {
    id: WidgetId,
    label: Cow<'static, str>,
}

struct OnToggle<H>(H);

impl<S, H: Callback<S, bool>> EventHandler<S> for OnToggle<H> {
    fn handle(&mut self, state: &mut S, event: &Event) {
        if let Event::Toggle(_, checked) = event {
            self.0.call(state, *checked);
        }
    }
}

impl<S, H: Callback<S, bool> + 'static> View<S> for Checkbox<H> {
    type Retained = CheckboxState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<CheckboxState> {
        let id = cx.create(
            parent,
            Properties::Checkbox {
                label: &self.label,
                checked: self.checked,
            },
        )?;
        cx.set_value(id, Value::Bool(self.checked));
        install(cx, id, H::ACTIVE, OnToggle(self.handler));
        Ok(CheckboxState {
            id,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut CheckboxState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if self.label != old.label {
            cx.update(old.id, Change::Label(&self.label))?;
            old.label = self.label;
        }
        if *cx.value(old.id) != Value::Bool(self.checked) {
            cx.update(old.id, Change::Checked(self.checked))?;
            cx.set_value(old.id, Value::Bool(self.checked));
        }
        install(cx, old.id, H::ACTIVE, OnToggle(self.handler));
        Ok(())
    }

    fn roots(retained: &CheckboxState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

pub struct Slider<H = NoHandler> {
    value: f64,
    range: SliderRange,
    label: Cow<'static, str>,
    handler: H,
}

/// A horizontal slider. The value is clamped to the range; the default step
/// is a hundredth of the range.
pub fn slider(value: f64, range: RangeInclusive<f64>) -> Slider {
    let (min, max) = range.into_inner();
    assert!(
        min.is_finite() && max.is_finite() && min < max,
        "slider range must be finite and nonempty"
    );
    Slider {
        value,
        range: SliderRange {
            min,
            max,
            step: (max - min) / 100.0,
        },
        label: Cow::Borrowed(""),
        handler: NoHandler,
    }
}

impl<H> Slider<H> {
    pub fn step(mut self, step: f64) -> Self {
        assert!(
            step.is_finite() && step > 0.0,
            "slider step must be positive and finite"
        );
        self.range.step = step;
        self
    }

    /// The accessible name; it is not drawn.
    pub fn label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn on_change<S, F: FnMut(&mut S, f64) + 'static>(self, handler: F) -> Slider<Handler<F>> {
        Slider {
            value: self.value,
            range: self.range,
            label: self.label,
            handler: Handler(handler),
        }
    }

    fn clamped(&self) -> f64 {
        if self.value.is_nan() {
            self.range.min
        } else {
            self.value.clamp(self.range.min, self.range.max)
        }
    }
}

pub struct SliderState {
    id: WidgetId,
    range: SliderRange,
    label: Cow<'static, str>,
}

struct OnSlide<H>(H);

impl<S, H: Callback<S, f64>> EventHandler<S> for OnSlide<H> {
    fn handle(&mut self, state: &mut S, event: &Event) {
        if let Event::Slide(_, value) = event {
            self.0.call(state, *value);
        }
    }
}

impl<S, H: Callback<S, f64> + 'static> View<S> for Slider<H> {
    type Retained = SliderState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<SliderState> {
        let value = self.clamped();
        let id = cx.create(
            parent,
            Properties::Slider {
                value,
                range: self.range,
                label: &self.label,
            },
        )?;
        cx.set_value(id, Value::Number(value));
        install(cx, id, H::ACTIVE, OnSlide(self.handler));
        Ok(SliderState {
            id,
            range: self.range,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut SliderState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        let value = self.clamped();
        if self.range != old.range {
            cx.update(old.id, Change::SliderRange(self.range))?;
            old.range = self.range;
        }
        if *cx.value(old.id) != Value::Number(value) {
            cx.update(old.id, Change::SliderValue(value))?;
            cx.set_value(old.id, Value::Number(value));
        }
        if self.label != old.label {
            cx.update(old.id, Change::AccessibleLabel(&self.label))?;
            old.label = self.label;
        }
        install(cx, old.id, H::ACTIVE, OnSlide(self.handler));
        Ok(())
    }

    fn roots(retained: &SliderState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

pub struct Progress {
    fraction: f64,
    label: Cow<'static, str>,
}

/// A determinate progress bar; `fraction` is clamped to `0.0..=1.0`.
pub fn progress(fraction: f64) -> Progress {
    let fraction = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    Progress {
        fraction,
        label: Cow::Borrowed(""),
    }
}

impl Progress {
    /// The accessible name; it is not drawn.
    pub fn label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = label.into();
        self
    }
}

pub struct ProgressState {
    id: WidgetId,
    fraction: f64,
    label: Cow<'static, str>,
}

impl<S> View<S> for Progress {
    type Retained = ProgressState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<ProgressState> {
        let id = cx.create(
            parent,
            Properties::Progress {
                fraction: self.fraction,
                label: &self.label,
            },
        )?;
        Ok(ProgressState {
            id,
            fraction: self.fraction,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut ProgressState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if self.fraction != old.fraction {
            cx.update(old.id, Change::Fraction(self.fraction))?;
            old.fraction = self.fraction;
        }
        if self.label != old.label {
            cx.update(old.id, Change::AccessibleLabel(&self.label))?;
            old.label = self.label;
        }
        Ok(())
    }

    fn roots(retained: &ProgressState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

pub struct Dropdown<H = NoHandler> {
    options: Vec<Cow<'static, str>>,
    selected: Option<usize>,
    label: Cow<'static, str>,
    handler: H,
}

/// A native drop-down list. An out-of-range `selected` shows no selection.
pub fn dropdown<T: Into<Cow<'static, str>>>(
    options: impl IntoIterator<Item = T>,
    selected: usize,
) -> Dropdown {
    let options: Vec<_> = options.into_iter().map(Into::into).collect();
    Dropdown {
        selected: (selected < options.len()).then_some(selected),
        options,
        label: Cow::Borrowed(""),
        handler: NoHandler,
    }
}

impl<H> Dropdown<H> {
    /// The accessible name; it is not drawn.
    pub fn label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn on_select<S, F: FnMut(&mut S, usize) + 'static>(
        self,
        handler: F,
    ) -> Dropdown<Handler<F>> {
        Dropdown {
            options: self.options,
            selected: self.selected,
            label: self.label,
            handler: Handler(handler),
        }
    }
}

pub struct DropdownState {
    id: WidgetId,
    options: Vec<Cow<'static, str>>,
    label: Cow<'static, str>,
}

struct OnSelect<H>(H);

impl<S, H: Callback<S, usize>> EventHandler<S> for OnSelect<H> {
    fn handle(&mut self, state: &mut S, event: &Event) {
        if let Event::Select(_, index) = event {
            self.0.call(state, *index);
        }
    }
}

impl<S, H: Callback<S, usize> + 'static> View<S> for Dropdown<H> {
    type Retained = DropdownState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<DropdownState> {
        let id = cx.create(
            parent,
            Properties::Dropdown {
                options: &self.options,
                selected: self.selected,
                label: &self.label,
            },
        )?;
        cx.set_value(id, Value::Index(self.selected));
        install(cx, id, H::ACTIVE, OnSelect(self.handler));
        Ok(DropdownState {
            id,
            options: self.options,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut DropdownState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        let options_changed = self.options != old.options;
        if options_changed {
            cx.update(old.id, Change::Options(&self.options))?;
            old.options = self.options;
        }
        // Replacing the options resets the native selection.
        if options_changed || *cx.value(old.id) != Value::Index(self.selected) {
            cx.update(old.id, Change::Selected(self.selected))?;
            cx.set_value(old.id, Value::Index(self.selected));
        }
        if self.label != old.label {
            cx.update(old.id, Change::AccessibleLabel(&self.label))?;
            old.label = self.label;
        }
        install(cx, old.id, H::ACTIVE, OnSelect(self.handler));
        Ok(())
    }

    fn roots(retained: &DropdownState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

/// An application-provided local image. A missing or undecodable file is an
/// error returned from the app's run.
pub struct Image {
    path: PathBuf,
    label: Cow<'static, str>,
}

pub fn image(path: impl Into<PathBuf>) -> Image {
    Image {
        path: path.into(),
        label: Cow::Borrowed(""),
    }
}

impl Image {
    /// The accessible description; it is not drawn.
    pub fn label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = label.into();
        self
    }
}

pub struct ImageState {
    id: WidgetId,
    path: PathBuf,
    label: Cow<'static, str>,
}

impl<S> View<S> for Image {
    type Retained = ImageState;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<ImageState> {
        let id = cx.create(
            parent,
            Properties::Image {
                path: &self.path,
                label: &self.label,
            },
        )?;
        Ok(ImageState {
            id,
            path: self.path,
            label: self.label,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut ImageState,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if self.path != old.path {
            cx.update(old.id, Change::Image(&self.path))?;
            old.path = self.path;
        }
        if self.label != old.label {
            cx.update(old.id, Change::AccessibleLabel(&self.label))?;
            old.label = self.label;
        }
        Ok(())
    }

    fn roots(retained: &ImageState, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

/// A vertically scrolling native viewport. Its content is laid out at the
/// viewport's width and its own natural height.
pub struct Scroll<V> {
    content: V,
}

pub fn scroll<V>(content: V) -> Scroll<V> {
    Scroll { content }
}

pub struct ScrollState<R> {
    id: WidgetId,
    content: R,
}

impl<S, V: View<S>> View<S> for Scroll<V> {
    type Retained = ScrollState<V::Retained>;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        let id = cx.create(parent, Properties::Scroll)?;
        let content = self.content.build(cx, id)?;
        Ok(ScrollState { id, content })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        cx.rebuild_children(self.content, &mut old.content, old.id)
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    /// Fill the cross axis of a row or column, or the whole stack.
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    Row,
    Column,
    Stack,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroupStyle {
    pub axis: Axis,
    pub gap: f32,
    pub padding: f32,
    pub align: Align,
}

pub struct Group<C> {
    children: C,
    style: GroupStyle,
}

pub fn row<C>(children: C) -> Group<C> {
    group(children, Axis::Row)
}

pub fn column<C>(children: C) -> Group<C> {
    group(children, Axis::Column)
}

/// Children overlap in source order, later ones drawn above earlier ones.
/// The stack is as large as its largest child; `align` places each child.
pub fn stack<C>(children: C) -> Group<C> {
    group(children, Axis::Stack)
}

fn group<C>(children: C, axis: Axis) -> Group<C> {
    Group {
        children,
        style: GroupStyle {
            axis,
            gap: 0.0,
            padding: 0.0,
            align: Align::Start,
        },
    }
}

impl<C> Group<C> {
    /// Space between a row's or column's children; a stack ignores it.
    pub fn gap(mut self, gap: f32) -> Self {
        assert!(
            gap.is_finite() && gap >= 0.0,
            "gap must be nonnegative and finite"
        );
        self.style.gap = gap;
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        assert!(
            padding.is_finite() && padding >= 0.0,
            "padding must be nonnegative and finite"
        );
        self.style.padding = padding;
        self
    }

    /// Cross-axis alignment in a row or column; placement on both axes in a
    /// stack. Main-axis placement is unchanged.
    pub fn align(mut self, align: Align) -> Self {
        self.style.align = align;
        self
    }
}

/// The children of a row, column, or stack: a tuple of views or a keyed
/// collection.
pub trait Children<S> {
    type Retained;
    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained>;
    fn rebuild<B: Backend>(
        self,
        old: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<()>;
    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>);
}

macro_rules! tuple_children {
    ($($name:ident:$index:tt),+) => {
        impl<S, $($name: View<S>),+> Children<S> for ($($name,)+) {
            type Retained = ($($name::Retained,)+);

            fn build<BackendType: Backend>(self, cx: &mut BuildContext<'_, S, BackendType>, parent: WidgetId) -> Result<Self::Retained> {
                Ok(($(self.$index.build(cx, parent)?,)+))
            }

            fn rebuild<BackendType: Backend>(self, old: &mut Self::Retained, cx: &mut BuildContext<'_, S, BackendType>, parent: WidgetId) -> Result<()> {
                $(self.$index.rebuild(&mut old.$index, cx, parent)?;)+
                Ok(())
            }

            fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
                $($name::roots(&retained.$index, output);)+
            }
        }
    };
}

tuple_children!(A:0);
tuple_children!(A:0, B:1);
tuple_children!(A:0, B:1, C:2);
tuple_children!(A:0, B:1, C:2, D:3);
tuple_children!(A:0, B:1, C:2, D:3, E:4);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
tuple_children!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

impl<S, K, V> Children<S> for Keyed<K, V>
where
    K: Hash + Eq + Debug,
    V: View<S>,
{
    type Retained = Vec<(K, V::Retained)>;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        View::build(self, cx, parent)
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<()> {
        View::rebuild(self, old, cx, parent)
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        <Self as View<S>>::roots(retained, output)
    }
}

pub struct GroupState<C> {
    id: WidgetId,
    style: GroupStyle,
    children: C,
}

impl<S, C: Children<S>> View<S> for Group<C> {
    type Retained = GroupState<C::Retained>;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        let overlap = self.style.axis == Axis::Stack;
        let id = cx.create(parent, Properties::Group { overlap })?;
        cx.set_group(id, self.style);
        let children = self.children.build(cx, id)?;
        Ok(GroupState {
            id,
            style: self.style,
            children,
        })
    }

    fn rebuild<B: Backend>(
        self,
        old: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        _: WidgetId,
    ) -> Result<()> {
        if old.style != self.style {
            let overlap = self.style.axis == Axis::Stack;
            if overlap != (old.style.axis == Axis::Stack) {
                cx.update(old.id, Change::Overlap(overlap))?;
            }
            cx.set_group(old.id, self.style);
            old.style = self.style;
        }
        let structure = cx.structure();
        self.children.rebuild(&mut old.children, cx, old.id)?;
        if cx.structure() != structure {
            let mut order = Vec::new();
            C::roots(&old.children, &mut order);
            cx.order(old.id, &order)?;
        }
        Ok(())
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        output.push(retained.id);
    }
}

/// A homogeneous collection matched by caller-provided keys, unique among
/// siblings. Surviving keys keep their native widgets, also when reordered.
pub struct Keyed<K, V> {
    items: Vec<(K, V)>,
}

pub fn keyed<K, V>(items: impl IntoIterator<Item = (K, V)>) -> Keyed<K, V> {
    Keyed {
        items: items.into_iter().collect(),
    }
}

fn check_unique<K: Hash + Eq + Debug, V>(items: &[(K, V)]) -> Result<()> {
    let mut seen = HashSet::with_capacity(items.len());
    for (key, _) in items {
        if !seen.insert(key) {
            return Err(format!("duplicate key {key:?} among keyed siblings").into());
        }
    }
    Ok(())
}

impl<S, K: Hash + Eq + Debug, V: View<S>> View<S> for Keyed<K, V> {
    type Retained = Vec<(K, V::Retained)>;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        check_unique(&self.items)?;
        let mut retained = Vec::with_capacity(self.items.len());
        for (key, view) in self.items {
            retained.push((key, view.build(cx, parent)?));
        }
        Ok(retained)
    }

    fn rebuild<B: Backend>(
        self,
        retained: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<()> {
        check_unique(&self.items)?;
        let mut previous: HashMap<K, (usize, V::Retained)> = retained
            .drain(..)
            .enumerate()
            .map(|(index, (key, item))| (key, (index, item)))
            .collect();
        // Survivors moved when their previous positions stop increasing.
        let mut last = None;
        let mut moved = false;
        for (key, view) in self.items {
            let item = match previous.remove(&key) {
                Some((index, mut item)) => {
                    moved |= last.is_some_and(|last| index < last);
                    last = Some(index);
                    view.rebuild(&mut item, cx, parent)?;
                    item
                }
                None => view.build(cx, parent)?,
            };
            retained.push((key, item));
        }
        for (_, item) in previous.values() {
            remove_view::<S, V, B>(item, cx);
        }
        if moved {
            cx.mark_moved();
        }
        Ok(())
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        for (_, item) in retained {
            V::roots(item, output);
        }
    }
}

impl<S, V: View<S>> View<S> for Option<V> {
    type Retained = Option<V::Retained>;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        self.map(|view| view.build(cx, parent)).transpose()
    }

    fn rebuild<B: Backend>(
        self,
        retained: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<()> {
        match (self, retained.as_mut()) {
            (Some(view), Some(item)) => view.rebuild(item, cx, parent)?,
            (Some(view), None) => *retained = Some(view.build(cx, parent)?),
            (None, Some(_)) => {
                if let Some(item) = retained.take() {
                    remove_view::<S, V, B>(&item, cx);
                }
            }
            (None, None) => {}
        }
        Ok(())
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        if let Some(item) = retained {
            V::roots(item, output);
        }
    }
}

/// One of two view types; switching variants replaces the subtree.
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<S, L: View<S>, R: View<S>> View<S> for Either<L, R> {
    type Retained = Either<L::Retained, R::Retained>;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        Ok(match self {
            Either::Left(view) => Either::Left(view.build(cx, parent)?),
            Either::Right(view) => Either::Right(view.build(cx, parent)?),
        })
    }

    fn rebuild<B: Backend>(
        self,
        retained: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<()> {
        match (self, &mut *retained) {
            (Either::Left(view), Either::Left(item)) => view.rebuild(item, cx, parent),
            (Either::Right(view), Either::Right(item)) => view.rebuild(item, cx, parent),
            (view, _) => {
                Self::remove(retained, cx);
                *retained = view.build(cx, parent)?;
                Ok(())
            }
        }
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        match retained {
            Either::Left(item) => L::roots(item, output),
            Either::Right(item) => R::roots(item, output),
        }
    }
}

impl<L, R> Either<L, R> {
    fn remove<S, B: Backend>(
        retained: &Either<L::Retained, R::Retained>,
        cx: &mut BuildContext<'_, S, B>,
    ) where
        L: View<S>,
        R: View<S>,
    {
        match retained {
            Either::Left(item) => remove_view::<S, L, B>(item, cx),
            Either::Right(item) => remove_view::<S, R, B>(item, cx),
        }
    }
}

/// A view whose root widgets take a share of a row's or column's spare
/// main-axis space.
pub struct Grow<V> {
    view: V,
    factor: f32,
}

impl<S, V: View<S>> View<S> for Grow<V> {
    type Retained = V::Retained;

    fn build<B: Backend>(
        self,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<Self::Retained> {
        let retained = self.view.build(cx, parent)?;
        apply_grow::<S, V, B>(&retained, self.factor, cx);
        Ok(retained)
    }

    fn rebuild<B: Backend>(
        self,
        retained: &mut Self::Retained,
        cx: &mut BuildContext<'_, S, B>,
        parent: WidgetId,
    ) -> Result<()> {
        self.view.rebuild(retained, cx, parent)?;
        apply_grow::<S, V, B>(retained, self.factor, cx);
        Ok(())
    }

    fn roots(retained: &Self::Retained, output: &mut Vec<WidgetId>) {
        V::roots(retained, output)
    }
}

fn apply_grow<S, V: View<S>, B: Backend>(
    retained: &V::Retained,
    factor: f32,
    cx: &mut BuildContext<'_, S, B>,
) {
    let mut roots = Vec::new();
    V::roots(retained, &mut roots);
    for root in roots {
        cx.set_grow(root, factor);
    }
}

/// Layout modifiers available on every view.
pub trait Modifiers: Sized {
    /// Take `factor` shares of the parent row's or column's spare space.
    fn grow(self, factor: f32) -> Grow<Self> {
        assert!(
            factor.is_finite() && factor >= 0.0,
            "grow factor must be nonnegative and finite"
        );
        Grow { view: self, factor }
    }
}

impl Modifiers for Text {}
impl<H> Modifiers for Button<H> {}
impl<I, U> Modifiers for TextInput<I, U> {}
impl<H> Modifiers for Checkbox<H> {}
impl<H> Modifiers for Slider<H> {}
impl Modifiers for Progress {}
impl<H> Modifiers for Dropdown<H> {}
impl Modifiers for Image {}
impl<V> Modifiers for Scroll<V> {}
impl<C> Modifiers for Group<C> {}
impl<K, V> Modifiers for Keyed<K, V> {}
impl<V> Modifiers for Option<V> {}
impl<L, R> Modifiers for Either<L, R> {}
impl<V> Modifiers for Grow<V> {}

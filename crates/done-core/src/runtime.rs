use crate::layout::{apply_frames, LayoutTree};
use crate::view::{Align, Axis, GroupStyle};
use crate::{
    Backend, Change, Constraints, Event, Kind, Measurement, Properties, Rect, Result, Size, View,
    WidgetId, WindowOptions,
};
use std::collections::HashMap;

/// Adapts a widget's typed callbacks to the events of that widget.
pub(crate) trait EventHandler<S> {
    fn handle(&mut self, state: &mut S, event: &Event);
}

/// The value a stateful control currently shows natively. Native observations
/// update it before reconciliation, so an accepted edit needs no setter.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    None,
    Text(String),
    Bool(bool),
    Number(f64),
    Index(Option<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Arrange {
    Leaf,
    Flex,
    Stack { align: Align, padding: f32 },
    Scroll,
}

pub(crate) struct Node {
    pub parent: Option<WidgetId>,
    pub children: Vec<WidgetId>,
    pub arrange: Arrange,
    pub style: taffy::Style,
    pub cache: taffy::Cache,
    pub layout: taffy::Layout,
    pub measured: Vec<(Constraints, Measurement)>,
    pub frame: Option<Rect>,
    pub value: Value,
}

impl Node {
    fn new(parent: Option<WidgetId>, kind: Kind) -> Self {
        let mut style = taffy::Style::DEFAULT;
        let arrange = match kind {
            // The root view fills the window.
            Kind::Window => Arrange::Stack {
                align: Align::Stretch,
                padding: 0.0,
            },
            Kind::Group => Arrange::Flex,
            Kind::Scroll => {
                style.overflow.y = taffy::Overflow::Scroll;
                Arrange::Scroll
            }
            _ => Arrange::Leaf,
        };
        Self {
            parent,
            children: Vec::new(),
            arrange,
            style,
            cache: taffy::Cache::new(),
            layout: taffy::Layout::with_order(0),
            measured: Vec::new(),
            frame: None,
            value: Value::None,
        }
    }
}

struct Renderer<S> {
    nodes: HashMap<WidgetId, Node>,
    handlers: HashMap<WidgetId, Box<dyn EventHandler<S>>>,
    next_id: u64,
    layout_dirty: bool,
    /// Bumped by every creation and removal; groups compare it to decide
    /// whether native child order needs checking.
    structure: u64,
    minimum: Option<Size>,
}

impl<S> Renderer<S> {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            handlers: HashMap::new(),
            next_id: 1,
            layout_dirty: true,
            structure: 0,
            minimum: None,
        }
    }

    fn create<B: Backend>(
        &mut self,
        backend: &mut B,
        parent: Option<WidgetId>,
        properties: Properties<'_>,
    ) -> Result<WidgetId> {
        let id = WidgetId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or("widget IDs exhausted")?;
        backend.create(id, parent, properties)?;
        self.nodes.insert(id, Node::new(parent, properties.kind()));
        if let Some(parent) = parent {
            self.nodes
                .get_mut(&parent)
                .expect("retained parent exists")
                .children
                .push(id);
            self.invalidate(parent, false);
        }
        self.structure += 1;
        Ok(id)
    }

    /// Clear cached layout from `id` up to the root, and the node's own native
    /// measurements when its size may have changed.
    fn invalidate(&mut self, id: WidgetId, measurement: bool) {
        if measurement {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.measured.clear();
            }
        }
        let mut next = Some(id);
        while let Some(id) = next {
            let node = self.nodes.get_mut(&id).expect("retained widget exists");
            node.cache.clear();
            next = node.parent;
        }
        self.layout_dirty = true;
    }

    fn collect(&self, id: WidgetId, output: &mut Vec<WidgetId>) {
        if let Some(node) = self.nodes.get(&id) {
            for &child in &node.children {
                self.collect(child, output);
            }
            output.push(id);
        }
    }

    /// Unregister the subtree's handlers, destroy native widgets children
    /// first, then release retained layout state.
    fn remove<B: Backend>(&mut self, backend: &mut B, id: WidgetId) {
        let mut subtree = Vec::new();
        self.collect(id, &mut subtree);
        if subtree.is_empty() {
            return;
        }
        for id in &subtree {
            self.handlers.remove(id);
        }
        for &id in &subtree {
            backend.destroy(id);
        }
        let parent = self.nodes[&id].parent;
        for id in &subtree {
            self.nodes.remove(id);
        }
        if let Some(parent) = parent.filter(|parent| self.nodes.contains_key(parent)) {
            self.nodes
                .get_mut(&parent)
                .expect("retained parent exists")
                .children
                .retain(|child| *child != id);
            self.invalidate(parent, false);
        }
        self.structure += 1;
    }

    fn layout<B: Backend>(&mut self, backend: &mut B, root: WidgetId, size: Size) -> Result<()> {
        if !self.layout_dirty {
            return Ok(());
        }
        let mut tree = LayoutTree::new(&mut self.nodes, backend);
        let minimum = tree.minimum(root)?;
        tree.perform(root, size)?;
        if self.minimum != Some(minimum) {
            backend.update(root, Change::MinimumSize(minimum))?;
            self.minimum = Some(minimum);
        }
        apply_frames(&mut self.nodes, backend, root)?;
        self.layout_dirty = false;
        Ok(())
    }
}

pub struct BuildContext<'a, S, B> {
    renderer: &'a mut Renderer<S>,
    backend: &'a mut B,
}

impl<S, B: Backend> BuildContext<'_, S, B> {
    pub(crate) fn create(
        &mut self,
        parent: WidgetId,
        properties: Properties<'_>,
    ) -> Result<WidgetId> {
        self.renderer.create(self.backend, Some(parent), properties)
    }

    pub(crate) fn update(&mut self, id: WidgetId, change: Change<'_>) -> Result<()> {
        self.backend.update(id, change)?;
        if change.affects_size() {
            self.renderer.invalidate(id, true);
        }
        Ok(())
    }

    pub(crate) fn remove(&mut self, id: WidgetId) {
        self.renderer.remove(self.backend, id);
    }

    pub(crate) fn set_handler(&mut self, id: WidgetId, handler: impl EventHandler<S> + 'static) {
        self.renderer.handlers.insert(id, Box::new(handler));
    }

    pub(crate) fn clear_handler(&mut self, id: WidgetId) {
        self.renderer.handlers.remove(&id);
    }

    pub(crate) fn value(&self, id: WidgetId) -> &Value {
        &self.renderer.nodes[&id].value
    }

    pub(crate) fn set_value(&mut self, id: WidgetId, value: Value) {
        self.renderer
            .nodes
            .get_mut(&id)
            .expect("retained widget exists")
            .value = value;
    }

    pub(crate) fn set_group(&mut self, id: WidgetId, group: GroupStyle) {
        let node = self
            .renderer
            .nodes
            .get_mut(&id)
            .expect("retained group exists");
        let length = taffy::LengthPercentage::length;
        node.style.padding = taffy::Rect {
            left: length(group.padding),
            right: length(group.padding),
            top: length(group.padding),
            bottom: length(group.padding),
        };
        node.arrange = match group.axis {
            Axis::Row | Axis::Column => {
                node.style.display = taffy::Display::Flex;
                node.style.flex_direction = if group.axis == Axis::Row {
                    taffy::FlexDirection::Row
                } else {
                    taffy::FlexDirection::Column
                };
                node.style.gap = taffy::Size {
                    width: length(group.gap),
                    height: length(group.gap),
                };
                // Safe alignment falls back to start instead of pushing an
                // overflowing child past the leading edge.
                node.style.align_items = Some(match group.align {
                    Align::Start => taffy::AlignItems::START,
                    Align::Center => taffy::AlignItems::SAFE_CENTER,
                    Align::End => taffy::AlignItems::SAFE_END,
                    Align::Stretch => taffy::AlignItems::STRETCH,
                });
                Arrange::Flex
            }
            Axis::Stack => Arrange::Stack {
                align: group.align,
                padding: group.padding,
            },
        };
        self.renderer.invalidate(id, false);
    }

    pub(crate) fn set_grow(&mut self, id: WidgetId, grow: f32) {
        let node = self
            .renderer
            .nodes
            .get_mut(&id)
            .expect("retained widget exists");
        if node.style.flex_grow != grow {
            node.style.flex_grow = grow;
            self.renderer.invalidate(id, false);
        }
    }

    pub(crate) fn structure(&self) -> u64 {
        self.renderer.structure
    }

    /// Children moved without creation or removal; parents must check order.
    pub(crate) fn mark_moved(&mut self) {
        self.renderer.structure += 1;
    }

    /// Bring native child order in line with the description, moving survivors.
    pub(crate) fn order(&mut self, parent: WidgetId, order: &[WidgetId]) -> Result<()> {
        let node = &self.renderer.nodes[&parent];
        if node.children != order {
            debug_assert!(
                node.children.len() == order.len()
                    && order.iter().all(|child| node.children.contains(child)),
                "an order lists exactly the parent's children"
            );
            self.backend.update(parent, Change::Order(order))?;
            self.renderer
                .nodes
                .get_mut(&parent)
                .expect("retained parent exists")
                .children = order.to_vec();
            self.renderer.invalidate(parent, false);
        }
        Ok(())
    }

    /// Rebuild `view` and then fix `parent`'s child order if anything below was
    /// created or removed.
    pub(crate) fn rebuild_children<V: View<S>>(
        &mut self,
        view: V,
        retained: &mut V::Retained,
        parent: WidgetId,
    ) -> Result<()> {
        let structure = self.structure();
        view.rebuild(retained, self, parent)?;
        if self.structure() != structure {
            let mut order = Vec::new();
            V::roots(retained, &mut order);
            self.order(parent, &order)?;
        }
        Ok(())
    }
}

/// One controller per native event loop. Native callbacks must not reenter it.
pub struct Driver<S, F, V: View<S>> {
    state: S,
    view: F,
    options: WindowOptions,
    renderer: Renderer<S>,
    root: Option<WidgetId>,
    retained: Option<V::Retained>,
    closed: bool,
}

impl<S, F: Fn(&S) -> V, V: View<S>> Driver<S, F, V> {
    pub fn new(options: WindowOptions, state: S, view: F) -> Self {
        Self {
            state,
            view,
            options,
            renderer: Renderer::new(),
            root: None,
            retained: None,
            closed: false,
        }
    }

    pub fn state(&self) -> &S {
        &self.state
    }

    pub fn dispatch<B: Backend>(&mut self, backend: &mut B, events: &[Event]) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let result = self.flush(backend, events);
        if result.is_err() {
            self.close(backend);
        }
        result
    }

    fn flush<B: Backend>(&mut self, backend: &mut B, events: &[Event]) -> Result<()> {
        let mut changed = false;
        for event in events {
            match event {
                Event::Started => {
                    if self.root.is_some() {
                        return Err("app already started".into());
                    }
                    let root =
                        self.renderer
                            .create(backend, None, Properties::Window(&self.options))?;
                    self.root = Some(root);
                    let mut cx = BuildContext {
                        renderer: &mut self.renderer,
                        backend,
                    };
                    self.retained = Some((self.view)(&self.state).build(&mut cx, root)?);
                }
                Event::Resized(size) => {
                    if self.options.size != *size {
                        self.options.size = *size;
                        self.renderer.layout_dirty = true;
                    }
                }
                Event::AppearanceChanged => {
                    for node in self.renderer.nodes.values_mut() {
                        node.measured.clear();
                        node.cache.clear();
                    }
                    self.renderer.layout_dirty = true;
                }
                Event::Close => {
                    self.close(backend);
                    return Ok(());
                }
                _ => {
                    let id = event.target().expect("widget event has a target");
                    // A queued event for a removed widget is stale.
                    let Some(node) = self.renderer.nodes.get_mut(&id) else {
                        continue;
                    };
                    let observed = match event {
                        Event::Input(_, value) => Some(Value::Text(value.clone())),
                        Event::Toggle(_, checked) => Some(Value::Bool(*checked)),
                        Event::Slide(_, value) => Some(Value::Number(*value)),
                        Event::Select(_, index) => Some(Value::Index(Some(*index))),
                        _ => None,
                    };
                    let is_value = observed.is_some();
                    if let Some(value) = observed {
                        node.value = value;
                    }
                    if let Some(handler) = self.renderer.handlers.get_mut(&id) {
                        handler.handle(&mut self.state, event);
                        changed = true;
                    } else if is_value {
                        // Controlled: an edit the state does not accept is reverted.
                        changed = true;
                    }
                }
            }
        }
        if changed {
            let root = self.root.ok_or("app has not started")?;
            let mut cx = BuildContext {
                renderer: &mut self.renderer,
                backend,
            };
            cx.rebuild_children(
                (self.view)(&self.state),
                self.retained.as_mut().ok_or("app has not started")?,
                root,
            )?;
        }
        if let Some(root) = self.root {
            self.renderer.layout(backend, root, self.options.size)?;
        }
        Ok(())
    }

    fn close<B: Backend>(&mut self, backend: &mut B) {
        if let Some(root) = self.root.take() {
            self.renderer.remove(backend, root);
        }
        self.retained = None;
        self.closed = true;
    }
}

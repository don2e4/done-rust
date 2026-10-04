//! Taffy over core's retained widget tree.
//!
//! Core implements taffy's low-level tree traits rather than mirroring every
//! node into `TaffyTree`: flex groups use taffy's flexbox algorithm, while a
//! stack and a scroll viewport use the two small algorithms below, which
//! taffy's flexbox-only feature set cannot express. Leaves are measured by the
//! native backend. Frames stay unrounded; backends round in device units.

use crate::runtime::{Arrange, Node};
use crate::view::Align;
use crate::{Backend, Constraints, Error, Measurement, Rect, Result, Size, WidgetId};
use std::collections::HashMap;
use taffy::{
    compute_cached_layout, compute_flexbox_layout, compute_leaf_layout, AvailableSpace, CacheTree,
    Layout, LayoutFlexboxContainer, LayoutInput, LayoutOutput, LayoutPartialTree, Line, NodeId,
    Point, RequestedAxis, RunMode, SizingMode, Style, TraversePartialTree,
};

type Dimensions = taffy::Size<f32>;
type Known = taffy::Size<Option<f32>>;
type Space = taffy::Size<AvailableSpace>;

/// Native measurements kept per leaf, keyed by the constraint they answered.
const MEASUREMENTS_PER_LEAF: usize = 4;

pub(crate) struct LayoutTree<'a, B> {
    nodes: &'a mut HashMap<WidgetId, Node>,
    backend: &'a mut B,
    error: Option<Error>,
}

fn node_id(id: WidgetId) -> NodeId {
    NodeId::from(id.0)
}

fn widget_id(node: NodeId) -> WidgetId {
    WidgetId(u64::from(node))
}

fn input(run_mode: RunMode, known: Known, parent: Known, available: Space) -> LayoutInput {
    LayoutInput {
        run_mode,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: known,
        known_dimensions_are_definite: taffy::Size {
            width: true,
            height: true,
        },
        parent_size: parent,
        available_space: available,
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

impl<'a, B: Backend> LayoutTree<'a, B> {
    pub(crate) fn new(nodes: &'a mut HashMap<WidgetId, Node>, backend: &'a mut B) -> Self {
        Self {
            nodes,
            backend,
            error: None,
        }
    }

    fn node(&self, node: NodeId) -> &Node {
        &self.nodes[&widget_id(node)]
    }

    fn node_mut(&mut self, node: NodeId) -> &mut Node {
        self.nodes
            .get_mut(&widget_id(node))
            .expect("layout node exists")
    }

    fn finish<T>(&mut self, value: T) -> Result<T> {
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok(value),
        }
    }

    /// The smallest size the root's content supports.
    pub(crate) fn minimum(&mut self, root: WidgetId) -> Result<Size> {
        let space = taffy::Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::MinContent,
        };
        let output = self.compute_child_layout(
            node_id(root),
            input(RunMode::ComputeSize, Known::NONE, Known::NONE, space),
        );
        self.finish(Size {
            width: output.size.width,
            height: output.size.height,
        })
    }

    /// Lay out the root at exactly `size`.
    pub(crate) fn perform(&mut self, root: WidgetId, size: Size) -> Result<()> {
        let known = Known {
            width: Some(size.width),
            height: Some(size.height),
        };
        let space = known.map(|dimension| AvailableSpace::Definite(dimension.unwrap_or(0.0)));
        self.compute_child_layout(
            node_id(root),
            input(RunMode::PerformLayout, known, known, space),
        );
        self.finish(())
    }

    fn leaf(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let id = widget_id(node);
        let Self {
            nodes,
            backend,
            error,
        } = self;
        let Node {
            style, measured, ..
        } = nodes.get_mut(&id).expect("layout node exists");
        compute_leaf_layout(
            inputs,
            &*style,
            |_, _| 0.0,
            |known, available| match native_size(&mut **backend, measured, id, known, available) {
                Ok(size) => size,
                Err(failure) => {
                    error.get_or_insert(failure);
                    Dimensions::ZERO
                }
            },
        )
    }

    /// Children overlap in source order. Unstretched children keep their own
    /// size and are placed by `align` on both axes inside the padding.
    fn stack(
        &mut self,
        node: NodeId,
        inputs: LayoutInput,
        align: Align,
        padding: f32,
    ) -> LayoutOutput {
        let children = self.node(node).children.clone();
        let inset = 2.0 * padding;
        let known = inputs.known_dimensions;
        let inner_known = known.map(|dimension| dimension.map(|value| (value - inset).max(0.0)));
        let inner_space = inputs.available_space.map(|space| match space {
            AvailableSpace::Definite(value) => AvailableSpace::Definite((value - inset).max(0.0)),
            other => other,
        });
        let stretch = align == Align::Stretch;
        let mut content = Dimensions::ZERO;
        let mut sizes = Vec::with_capacity(children.len());
        for &child in &children {
            let child_known = if stretch { inner_known } else { Known::NONE };
            let size = self
                .compute_child_layout(
                    node_id(child),
                    input(RunMode::ComputeSize, child_known, inner_known, inner_space),
                )
                .size;
            content.width = content.width.max(size.width);
            content.height = content.height.max(size.height);
            sizes.push(size);
        }
        let size = Dimensions {
            width: known.width.unwrap_or(content.width + inset),
            height: known.height.unwrap_or(content.height + inset),
        };
        if inputs.run_mode != RunMode::PerformLayout {
            return LayoutOutput::from_outer_size(size);
        }
        let inner = Dimensions {
            width: (size.width - inset).max(0.0),
            height: (size.height - inset).max(0.0),
        };
        // Like the flex groups' safe alignment: never place before the padding.
        let offset = |free: f32| {
            padding
                + match align {
                    Align::Start | Align::Stretch => 0.0,
                    Align::Center => free.max(0.0) / 2.0,
                    Align::End => free.max(0.0),
                }
        };
        for (order, (&child, measured)) in children.iter().zip(sizes).enumerate() {
            let target = if stretch {
                inner
            } else {
                Dimensions {
                    width: measured.width.min(inner.width),
                    height: measured.height.min(inner.height),
                }
            };
            let output = self.compute_child_layout(
                node_id(child),
                input(
                    RunMode::PerformLayout,
                    target.map(Some),
                    inner.map(Some),
                    inner.map(AvailableSpace::Definite),
                ),
            );
            let mut layout = Layout::with_order(order as u32);
            layout.size = output.size;
            layout.location = Point {
                x: offset(inner.width - output.size.width),
                y: offset(inner.height - output.size.height),
            };
            self.set_unrounded_layout(node_id(child), &layout);
        }
        LayoutOutput::from_outer_size(size)
    }

    /// Content is laid out at the viewport width, less the backend's scroll bar
    /// gutter, with unconstrained height.
    /// Its preferred size is the content's; under a min-content height
    /// constraint the viewport may shrink to nothing, as its `overflow` style
    /// already makes its automatic minimum zero for flex parents.
    fn scroll(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let children = self.node(node).children.clone();
        let gutter = self.backend.scrollbar_gutter();
        let known = inputs.known_dimensions;
        let content_input = |run_mode, width: Option<f32>, space: AvailableSpace| {
            let known = Known {
                width,
                height: None,
            };
            let space = taffy::Size {
                width: space,
                height: AvailableSpace::MaxContent,
            };
            input(run_mode, known, known, space)
        };
        let size = match (known.width, known.height) {
            (Some(width), Some(height)) => Dimensions { width, height },
            _ => {
                let content_width = known.width.map(|width| (width - gutter).max(0.0));
                let space = match content_width {
                    Some(width) => AvailableSpace::Definite(width),
                    None => match inputs.available_space.width {
                        AvailableSpace::Definite(width) => {
                            AvailableSpace::Definite((width - gutter).max(0.0))
                        }
                        other => other,
                    },
                };
                let mut content = Dimensions::ZERO;
                for &child in &children {
                    let size = self
                        .compute_child_layout(
                            node_id(child),
                            content_input(RunMode::ComputeSize, content_width, space),
                        )
                        .size;
                    content.width = content.width.max(size.width);
                    content.height = content.height.max(size.height);
                }
                Dimensions {
                    width: known.width.unwrap_or(content.width + gutter),
                    height: known.height.unwrap_or(match inputs.available_space.height {
                        AvailableSpace::MinContent => 0.0,
                        _ => content.height,
                    }),
                }
            }
        };
        if inputs.run_mode != RunMode::PerformLayout {
            return LayoutOutput::from_outer_size(size);
        }
        let content_width = (size.width - gutter).max(0.0);
        for (order, &child) in children.iter().enumerate() {
            let output = self.compute_child_layout(
                node_id(child),
                content_input(
                    RunMode::PerformLayout,
                    Some(content_width),
                    AvailableSpace::Definite(content_width),
                ),
            );
            let mut layout = Layout::with_order(order as u32);
            layout.size = output.size;
            self.set_unrounded_layout(node_id(child), &layout);
        }
        LayoutOutput::from_outer_size(size)
    }
}

fn native_size<B: Backend>(
    backend: &mut B,
    cache: &mut Vec<(Constraints, Measurement)>,
    id: WidgetId,
    known: Known,
    available: Space,
) -> Result<Dimensions> {
    if let (Some(width), Some(height)) = (known.width, known.height) {
        return Ok(Dimensions { width, height });
    }
    let mut measure = |constraints: Constraints| -> Result<Measurement> {
        if let Some((_, measurement)) = cache.iter().find(|(cached, _)| *cached == constraints) {
            return Ok(*measurement);
        }
        let measurement = backend.measure(id, constraints)?;
        if cache.len() == MEASUREMENTS_PER_LEAF {
            cache.remove(0);
        }
        cache.push((constraints, measurement));
        Ok(measurement)
    };
    // At or above its natural size in one dimension a control needs its natural
    // size in the other; only a smaller given dimension, such as a narrow
    // width for wrapped text, needs another native measurement.
    let natural = measure(Constraints::default())?;
    let measurement = match (known.width, known.height) {
        (Some(width), None) if width < natural.preferred.width => measure(Constraints {
            width: Some(width),
            height: None,
        })?,
        (None, Some(height)) if height < natural.preferred.height => measure(Constraints {
            width: None,
            height: Some(height),
        })?,
        _ => natural,
    };
    let width = match known.width {
        Some(width) => width,
        None => fit(
            measurement.minimum.width,
            measurement.preferred.width,
            available.width,
        ),
    };
    let height = match known.height {
        Some(height) => height,
        None => {
            let measurement = if known.width.is_none() && width < measurement.preferred.width {
                measure(Constraints {
                    width: Some(width),
                    height: None,
                })?
            } else {
                measurement
            };
            fit(
                measurement.minimum.height,
                measurement.preferred.height,
                available.height,
            )
        }
    };
    Ok(Dimensions { width, height })
}

fn fit(minimum: f32, preferred: f32, available: AvailableSpace) -> f32 {
    match available {
        AvailableSpace::MinContent => minimum,
        AvailableSpace::MaxContent => preferred,
        AvailableSpace::Definite(space) => preferred.min(space).max(minimum),
    }
}

/// Apply changed frames below `id`, parents before children.
pub(crate) fn apply_frames<B: Backend>(
    nodes: &mut HashMap<WidgetId, Node>,
    backend: &mut B,
    id: WidgetId,
) -> Result<()> {
    for index in 0..nodes[&id].children.len() {
        let child = nodes[&id].children[index];
        let node = nodes.get_mut(&child).expect("retained child exists");
        let rect = Rect {
            x: node.layout.location.x,
            y: node.layout.location.y,
            width: node.layout.size.width,
            height: node.layout.size.height,
        };
        if node.frame != Some(rect) {
            backend.set_frame(child, rect)?;
            node.frame = Some(rect);
        }
        apply_frames(nodes, backend, child)?;
    }
    Ok(())
}

pub(crate) struct ChildIter<'a>(std::slice::Iter<'a, WidgetId>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        self.0.next().map(|id| node_id(*id))
    }
}

impl<B: Backend> TraversePartialTree for LayoutTree<'_, B> {
    type ChildIter<'b>
        = ChildIter<'b>
    where
        Self: 'b;

    fn child_ids(&self, parent: NodeId) -> ChildIter<'_> {
        ChildIter(self.node(parent).children.iter())
    }

    fn child_count(&self, parent: NodeId) -> usize {
        self.node(parent).children.len()
    }

    fn get_child_id(&self, parent: NodeId, index: usize) -> NodeId {
        node_id(self.node(parent).children[index])
    }
}

impl<B: Backend> LayoutPartialTree for LayoutTree<'_, B> {
    type CoreContainerStyle<'b>
        = &'b Style
    where
        Self: 'b;
    type CustomIdent = String;

    fn get_core_container_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    fn set_unrounded_layout(&mut self, node: NodeId, layout: &Layout) {
        self.node_mut(node).layout = *layout;
    }

    fn compute_child_layout(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        compute_cached_layout(self, node, inputs, |tree, node, inputs| {
            match tree.node(node).arrange {
                Arrange::Flex => compute_flexbox_layout(tree, node, inputs),
                Arrange::Stack { align, padding } => tree.stack(node, inputs, align, padding),
                Arrange::Scroll => tree.scroll(node, inputs),
                Arrange::Leaf => tree.leaf(node, inputs),
            }
        })
    }
}

impl<B: Backend> CacheTree for LayoutTree<'_, B> {
    fn cache_get(&mut self, node: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.node_mut(node).cache.get(input)
    }

    fn cache_store(&mut self, node: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.node_mut(node).cache.store(input, output);
    }

    fn cache_clear(&mut self, node: NodeId) {
        self.node_mut(node).cache.clear();
    }
}

impl<B: Backend> LayoutFlexboxContainer for LayoutTree<'_, B> {
    type FlexboxContainerStyle<'b>
        = &'b Style
    where
        Self: 'b;
    type FlexboxItemStyle<'b>
        = &'b Style
    where
        Self: 'b;

    fn get_flexbox_container_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    fn get_flexbox_child_style(&self, child: NodeId) -> &Style {
        &self.node(child).style
    }
}

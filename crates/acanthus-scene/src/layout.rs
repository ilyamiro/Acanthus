use cosmic_text::FontSystem;
use std::sync::Arc;

use acanthus_core::{Rect, Signal, Vec2};
use acanthus_image::{Decoded, ImageHandle, ImageStatus, ImageStore};
use slotmap::{SecondaryMap, SlotMap};

use crate::node::{ImageStyle, TextStyle};
use crate::node::{Kind, Node, NodeId};
use crate::text::{Elide, ShapedText, TextParams, WrapMode};

/// A reference edge of a node for anchoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The horizontal centre line.
    HCenter,
    /// The vertical centre line.
    VCenter,
}

/// Anchoring of a node to its parent or to sibling edges, as in Qt's `anchors`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Anchors {
    /// Margin from the left reference, when anchored.
    pub left: Option<f32>,
    /// Margin from the right reference, when anchored.
    pub right: Option<f32>,
    /// Margin from the top reference, when anchored.
    pub top: Option<f32>,
    /// Margin from the bottom reference, when anchored.
    pub bottom: Option<f32>,
    /// Centre horizontally in the parent.
    pub h_center: bool,
    /// Centre vertically in the parent.
    pub v_center: bool,
    /// Offset applied to horizontal centring.
    pub h_offset: f32,
    /// Offset applied to vertical centring.
    pub v_offset: f32,
    /// Sibling edge the left side is anchored to instead of the parent's.
    pub left_ref: Option<(NodeId, Edge)>,
    /// Sibling edge the right side is anchored to instead of the parent's.
    pub right_ref: Option<(NodeId, Edge)>,
    /// Sibling edge the top side is anchored to instead of the parent's.
    pub top_ref: Option<(NodeId, Edge)>,
    /// Sibling edge the bottom side is anchored to instead of the parent's.
    pub bottom_ref: Option<(NodeId, Edge)>,
}

impl Anchors {
    /// Anchors all four sides to the parent with `margin`.
    pub fn fill(margin: f32) -> Self {
        Anchors {
            left: Some(margin),
            right: Some(margin),
            top: Some(margin),
            bottom: Some(margin),
            ..Default::default()
        }
    }

    /// Centres the node in the parent on both axes.
    pub fn center() -> Self {
        Anchors { h_center: true, v_center: true, ..Default::default() }
    }

    /// Anchors the left side to the parent with `margin`.
    pub fn left(self, margin: f32) -> Self {
        Anchors { left: Some(margin), ..self }
    }

    /// Anchors the right side to the parent with `margin`.
    pub fn right(self, margin: f32) -> Self {
        Anchors { right: Some(margin), ..self }
    }

    /// Anchors the top side to the parent with `margin`.
    pub fn top(self, margin: f32) -> Self {
        Anchors { top: Some(margin), ..self }
    }

    /// Anchors the bottom side to the parent with `margin`.
    pub fn bottom(self, margin: f32) -> Self {
        Anchors { bottom: Some(margin), ..self }
    }

    /// Centres horizontally in the parent.
    pub fn h_center(self) -> Self {
        Anchors { h_center: true, ..self }
    }

    /// Centres vertically in the parent.
    pub fn v_center(self) -> Self {
        Anchors { v_center: true, ..self }
    }

    /// Centres horizontally with an offset.
    pub fn h_center_offset(self, offset: f32) -> Self {
        Anchors { h_center: true, h_offset: offset, ..self }
    }

    /// Centres vertically with an offset.
    pub fn v_center_offset(self, offset: f32) -> Self {
        Anchors { v_center: true, v_offset: offset, ..self }
    }

    /// Anchors the left side to `edge` of `node` with `margin`.
    pub fn left_to(self, node: NodeId, edge: Edge, margin: f32) -> Self {
        Anchors { left: Some(margin), left_ref: Some((node, edge)), ..self }
    }

    /// Anchors the right side to `edge` of `node` with `margin`.
    pub fn right_to(self, node: NodeId, edge: Edge, margin: f32) -> Self {
        Anchors { right: Some(margin), right_ref: Some((node, edge)), ..self }
    }

    /// Anchors the top side to `edge` of `node` with `margin`.
    pub fn top_to(self, node: NodeId, edge: Edge, margin: f32) -> Self {
        Anchors { top: Some(margin), top_ref: Some((node, edge)), ..self }
    }

    /// Anchors the bottom side to `edge` of `node` with `margin`.
    pub fn bottom_to(self, node: NodeId, edge: Edge, margin: f32) -> Self {
        Anchors { bottom: Some(margin), bottom_ref: Some((node, edge)), ..self }
    }

    fn references(&self) -> impl Iterator<Item = NodeId> {
        [self.left_ref, self.right_ref, self.top_ref, self.bottom_ref].into_iter().flatten().map(|(id, _)| id)
    }
}

/// Space around a node inside its layout cell.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Margins {
    /// Left margin.
    pub left: f32,
    /// Top margin.
    pub top: f32,
    /// Right margin.
    pub right: f32,
    /// Bottom margin.
    pub bottom: f32,
}

impl Margins {
    /// The same margin on all four sides.
    pub fn all(v: f32) -> Self {
        Margins { left: v, top: v, right: v, bottom: v }
    }

    /// `x` on the left and right, `y` on the top and bottom.
    pub fn symmetric(x: f32, y: f32) -> Self {
        Margins { left: x, top: y, right: x, bottom: y }
    }

    fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// Size hints a node gives its parent's layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraints {
    /// Preferred width; `None` uses the content size.
    pub preferred_width: Option<f32>,
    /// Preferred height; `None` uses the content size.
    pub preferred_height: Option<f32>,
    /// Smallest width.
    pub min_width: f32,
    /// Smallest height.
    pub min_height: f32,
    /// Largest width.
    pub max_width: f32,
    /// Largest height.
    pub max_height: f32,
    /// Margins around the node.
    pub margins: Margins,
    /// Columns spanned in a grid.
    pub column_span: usize,
    /// Rows spanned in a grid.
    pub row_span: usize,
}

impl Default for Constraints {
    fn default() -> Self {
        Constraints {
            preferred_width: None,
            preferred_height: None,
            min_width: 0.0,
            min_height: 0.0,
            max_width: f32::INFINITY,
            max_height: f32::INFINITY,
            margins: Margins::default(),
            column_span: 1,
            row_span: 1,
        }
    }
}

impl Constraints {
    fn clamp_width(&self, w: f32) -> f32 {
        w.min(self.max_width).max(self.min_width)
    }

    fn clamp_height(&self, h: f32) -> f32 {
        h.min(self.max_height).max(self.min_height)
    }
}

/// How a node arranges its children.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Layout {
    /// Children position themselves (anchors or explicit rectangles).
    #[default]
    Free,
    /// Children in a horizontal row.
    Row {
        /// Gap between children.
        spacing: f32,
        /// Inset from the container's edges.
        padding: Vec2,
    },
    /// Children in a vertical column.
    Column {
        /// Gap between children.
        spacing: f32,
        /// Inset from the container's edges.
        padding: Vec2,
    },
    /// Children in a grid filled row by row.
    Grid {
        /// Number of columns.
        columns: usize,
        /// Column gap in `x`, row gap in `y`.
        spacing: Vec2,
        /// Inset from the container's edges.
        padding: Vec2,
    },
    /// Children in rows that wrap when the width is exhausted.
    Flow {
        /// Gap between children in `x`, between lines in `y`.
        spacing: Vec2,
        /// Inset from the container's edges.
        padding: Vec2,
    },
    /// Children stacked on top of each other, each filling the container.
    Stack {
        /// Inset from the container's edges.
        padding: Vec2,
    },
}

impl Layout {
    /// A row with `spacing` between children.
    pub fn row(spacing: f32) -> Self {
        Layout::Row { spacing, padding: Vec2::ZERO }
    }

    /// A column with `spacing` between children.
    pub fn column(spacing: f32) -> Self {
        Layout::Column { spacing, padding: Vec2::ZERO }
    }

    /// A grid with `columns` columns and the given gaps.
    pub fn grid(columns: usize, column_spacing: f32, row_spacing: f32) -> Self {
        Layout::Grid { columns: columns.max(1), spacing: Vec2::new(column_spacing, row_spacing), padding: Vec2::ZERO }
    }

    /// A wrapping flow with the given gap between items and between lines.
    pub fn flow(spacing: f32, line_spacing: f32) -> Self {
        Layout::Flow { spacing: Vec2::new(spacing, line_spacing), padding: Vec2::ZERO }
    }

    /// A stack of overlapping children.
    pub fn stack() -> Self {
        Layout::Stack { padding: Vec2::ZERO }
    }

    /// Sets the inset; has no effect on `Free`.
    pub fn padding(self, x: f32, y: f32) -> Self {
        let padding = Vec2::new(x, y);
        match self {
            Layout::Free => Layout::Free,
            Layout::Row { spacing, .. } => Layout::Row { spacing, padding },
            Layout::Column { spacing, .. } => Layout::Column { spacing, padding },
            Layout::Grid { columns, spacing, .. } => Layout::Grid { columns, spacing, padding },
            Layout::Flow { spacing, .. } => Layout::Flow { spacing, padding },
            Layout::Stack { .. } => Layout::Stack { padding },
        }
    }

    fn pad(&self) -> Vec2 {
        match self {
            Layout::Free => Vec2::ZERO,
            Layout::Row { padding, .. }
            | Layout::Column { padding, .. }
            | Layout::Grid { padding, .. }
            | Layout::Flow { padding, .. }
            | Layout::Stack { padding } => *padding,
        }
    }
}

fn inset(r: Rect, p: Vec2) -> Rect {
    Rect::new(r.x + p.x, r.y + p.y, (r.w - 2.0 * p.x).max(0.0), (r.h - 2.0 * p.y).max(0.0))
}

/// Alignment of a child inside the free space of its cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// At the start of the axis.
    Start,
    /// In the middle (the default).
    #[default]
    Center,
    /// At the end of the axis.
    End,
}

impl Align {
    fn offset(self, free: f32) -> f32 {
        match self {
            Align::Start => 0.0,
            Align::Center => (free / 2.0).round(),
            Align::End => free,
        }
    }
}

fn text_params(style: &TextStyle, width: Option<f32>, generation: u64) -> TextParams {
    let wrap = style.wrap.get();
    let elide = style.elide.get();
    TextParams {
        size: style.size.get(),
        weight: style.weight.get(),
        axes: style.axes,
        styled: style.styled,
        max_width: if wrap != WrapMode::NoWrap || elide != Elide::None { width } else { None },
        wrap,
        elide,
        max_lines: style.max_lines.get(),
        line_height: style.line_height.get(),
        letter_spacing: style.letter_spacing.get(),
        capitalization: style.capitalization.get(),
        generation,
    }
}

pub(crate) struct ImageEntry {
    pub source: String,
    pub target: Option<(u32, u32)>,
    pub handle: Option<ImageHandle>,
    pub ready: Option<acanthus_image::ImageId>,
    pub size: Vec2,
    pub clock: Option<FrameClock>,
}

pub(crate) struct FrameClock {
    pub frame: Signal<u32>,
    handle: acanthus_anim::AnimHandle,
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.handle.stop();
    }
}

struct Frames {
    frame: Signal<u32>,
    decoded: Arc<Decoded>,
    index: usize,
    elapsed_ms: f32,
}

impl acanthus_anim::Animation for Frames {
    fn start(&mut self) {}

    fn advance(&mut self, dt: f32) -> Option<f32> {
        self.elapsed_ms += dt * 1000.0;
        let count = self.decoded.frame_count();
        let mut moved = false;
        loop {
            let delay = self.decoded.delay_ms(self.index) as f32;
            if self.elapsed_ms < delay {
                break;
            }
            self.elapsed_ms -= delay;
            self.index = (self.index + 1) % count;
            moved = true;
        }
        if moved {
            self.frame.set(self.index as u32);
        }
        None
    }
}

fn frame_clock(decoded: &Arc<Decoded>) -> FrameClock {
    let frame = Signal::new(0u32);
    let handle =
        acanthus_anim::start(Frames { frame: frame.clone(), decoded: decoded.clone(), index: 0, elapsed_ms: 0.0 });
    FrameClock { frame, handle }
}

const UNBOUNDED: u32 = 16384;

pub(crate) struct LayoutCx<'a> {
    pub images: &'a mut SecondaryMap<NodeId, ImageEntry>,
    pub store: &'a ImageStore,
    pub scale: f32,
    pub nodes: &'a SlotMap<NodeId, Node>,
    pub texts: &'a mut SecondaryMap<NodeId, ShapedText>,
    pub fonts: &'a mut FontSystem,
    pub geometry: &'a mut SecondaryMap<NodeId, Rect>,
    pub implicit: SecondaryMap<NodeId, Vec2>,
    pub font_generation: u64,
}

struct Cell {
    child: NodeId,
    column: usize,
    row: usize,
    column_span: usize,
    row_span: usize,
}

struct GridMetrics {
    cells: Vec<Cell>,
    columns: Vec<f32>,
    rows: Vec<f32>,
}

impl LayoutCx<'_> {
    pub fn run(&mut self, root: NodeId, viewport: Rect) {
        self.geometry.clear();
        self.measure(root);
        self.arrange(root, viewport);
    }

    fn visible_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|c| self.nodes[*c].visible.get() || self.nodes[*c].laid_out_hidden)
    }

    fn measure(&mut self, id: NodeId) -> Vec2 {
        let children: Vec<NodeId> = self.nodes[id].children.clone();
        for child in &children {
            self.measure(*child);
        }
        let node = &self.nodes[id];
        let implicit = match &node.kind {
            Kind::Text(style) => {
                let params = text_params(style, node.width.as_ref().map(|w| w.get()), self.font_generation);
                self.shape_text(id, style, params);
                self.texts[id].size
            }
            Kind::Image(style) => self.resolve_image(
                id,
                style,
                node.width.as_ref().map(|w| w.get()),
                node.height.as_ref().map(|h| h.get()),
            ),
            _ => self.container_size(id),
        };
        self.implicit.insert(id, implicit);
        node.implicit_size.set(implicit);
        implicit
    }

    fn resolve_image(&mut self, id: NodeId, style: &ImageStyle, width: Option<f32>, height: Option<f32>) -> Vec2 {
        self.store.generation().get();
        let source = style.canvas.as_ref().map_or_else(|| style.source.get(), |canvas| canvas.source(self.scale));
        let physical = |v: f32| ((v * self.scale).ceil() as u32).max(1);
        let target = match (style.source_size.get(), width, height) {
            (Some(size), _, _) if size.x > 0.0 && size.y > 0.0 => Some((physical(size.x), physical(size.y))),
            (_, Some(w), Some(h)) => Some((physical(w), physical(h))),
            (_, Some(w), None) => Some((physical(w), UNBOUNDED)),
            (_, None, Some(h)) => Some((UNBOUNDED, physical(h))),
            _ => None,
        };
        let stale = self.images.get(id).is_none_or(|e| e.source != source || e.target != target);
        if stale {
            let handle = self.store.request(&source, target);
            self.images.insert(id, ImageEntry { source, target, handle, ready: None, size: Vec2::ZERO, clock: None });
        }
        let entry = &mut self.images[id];
        let status = entry.handle.as_ref().map_or(ImageStatus::Null, |h| self.store.status(h.id()));
        entry.ready = None;
        entry.size = Vec2::ZERO;
        if status == ImageStatus::Ready
            && let Some(handle) = &entry.handle
            && let Some(decoded) = self.store.get(handle.id())
        {
            let divisor = if entry.target.is_some() { self.scale } else { 1.0 };
            entry.ready = Some(handle.id());
            entry.size = Vec2::new(decoded.width as f32 / divisor, decoded.height as f32 / divisor);
            if style.animated.get() && decoded.frame_count() > 1 {
                if entry.clock.is_none() {
                    entry.clock = Some(frame_clock(&decoded));
                }
            } else {
                entry.clock = None;
            }
        } else {
            entry.clock = None;
        }
        style.status.set(status);
        if style.canvas.is_some() { Vec2::ZERO } else { entry.size }
    }

    fn shape_text(&mut self, id: NodeId, style: &TextStyle, params: TextParams) {
        match self.texts.get_mut(id) {
            Some(shaped) => shaped.update(self.fonts, &style.text, &style.family, params),
            None => {
                let shaped = ShapedText::new(self.fonts, &style.text, &style.family, params);
                self.texts.insert(id, shaped);
            }
        }
    }

    fn container_size(&self, id: NodeId) -> Vec2 {
        let node = &self.nodes[id];
        let layout = node.layout.get();
        let padding = layout.pad();
        let content = match layout {
            Layout::Free => return Vec2::ZERO,
            Layout::Row { spacing, .. } => self.linear_size(id, spacing, true),
            Layout::Column { spacing, .. } => self.linear_size(id, spacing, false),
            Layout::Grid { columns, spacing, .. } => {
                let m = self.grid_metrics(id, columns, spacing);
                Vec2::new(sum_with_spacing(&m.columns, spacing.x), sum_with_spacing(&m.rows, spacing.y))
            }
            Layout::Flow { spacing, .. } => {
                let limit = node.width.as_ref().map(|w| w.get() - 2.0 * padding.x);
                self.flow_lines(id, spacing, limit).1
            }
            Layout::Stack { .. } => {
                let mut size = Vec2::ZERO;
                for child in self.visible_children(id) {
                    size = size.max(self.outer(child));
                }
                size
            }
        };
        content + 2.0 * padding
    }

    fn linear_size(&self, id: NodeId, spacing: f32, horizontal: bool) -> Vec2 {
        let mut main: f32 = 0.0;
        let mut cross: f32 = 0.0;
        let mut count = 0;
        for child in self.visible_children(id) {
            let p = self.outer(child);
            let (m, c) = if horizontal { (p.x, p.y) } else { (p.y, p.x) };
            main += m;
            cross = cross.max(c);
            count += 1;
        }
        main += spacing * (count.max(1) - 1) as f32;
        if horizontal { Vec2::new(main, cross) } else { Vec2::new(cross, main) }
    }

    fn constraints(&self, id: NodeId) -> Constraints {
        self.nodes[id].constraints.get()
    }

    fn preferred(&self, id: NodeId) -> Vec2 {
        let node = &self.nodes[id];
        let c = node.constraints.get();
        let implicit = self.implicit.get(id).copied().unwrap_or_default();
        let w = node.width.as_ref().map(|w| w.get()).or(c.preferred_width).unwrap_or(implicit.x);
        let h = node.height.as_ref().map(|h| h.get()).or(c.preferred_height).unwrap_or(implicit.y);
        Vec2::new(c.clamp_width(w), c.clamp_height(h))
    }

    fn outer(&self, id: NodeId) -> Vec2 {
        let m = self.constraints(id).margins;
        self.preferred(id) + Vec2::new(m.horizontal(), m.vertical())
    }

    fn grid_metrics(&self, id: NodeId, columns: usize, spacing: Vec2) -> GridMetrics {
        let columns = columns.max(1);
        let mut occupied: Vec<Vec<bool>> = Vec::new();
        let mut cells = Vec::new();
        let (mut col, mut row) = (0usize, 0usize);
        for child in self.visible_children(id) {
            let c = self.constraints(child);
            let span = c.column_span.clamp(1, columns);
            let rows = c.row_span.max(1);
            loop {
                if occupied.len() < row + rows {
                    occupied.resize(row + rows, vec![false; columns]);
                }
                if col + span > columns {
                    col = 0;
                    row += 1;
                    continue;
                }
                let free = (row..row + rows).all(|r| (col..col + span).all(|k| !occupied[r][k]));
                if free {
                    break;
                }
                col += 1;
            }
            for line in occupied.iter_mut().skip(row).take(rows) {
                line[col..col + span].fill(true);
            }
            cells.push(Cell { child, column: col, row, column_span: span, row_span: rows });
            col += span;
        }
        let row_count = cells.iter().map(|c| c.row + c.row_span).max().unwrap_or(0);
        let mut widths = vec![0.0f32; if cells.is_empty() { 0 } else { columns }];
        let mut heights = vec![0.0f32; row_count];
        for cell in cells.iter().filter(|c| c.column_span == 1) {
            widths[cell.column] = widths[cell.column].max(self.outer(cell.child).x);
        }
        for cell in cells.iter().filter(|c| c.row_span == 1) {
            heights[cell.row] = heights[cell.row].max(self.outer(cell.child).y);
        }
        for cell in cells.iter().filter(|c| c.column_span > 1) {
            let range = cell.column..cell.column + cell.column_span;
            let have = sum_with_spacing(&widths[range.clone()], spacing.x);
            let missing = self.outer(cell.child).x - have;
            if missing > 0.0 {
                let share = missing / cell.column_span as f32;
                widths[range].iter_mut().for_each(|w| *w += share);
            }
        }
        for cell in cells.iter().filter(|c| c.row_span > 1) {
            let range = cell.row..cell.row + cell.row_span;
            let have = sum_with_spacing(&heights[range.clone()], spacing.y);
            let missing = self.outer(cell.child).y - have;
            if missing > 0.0 {
                let share = missing / cell.row_span as f32;
                heights[range].iter_mut().for_each(|h| *h += share);
            }
        }
        GridMetrics { cells, columns: widths, rows: heights }
    }

    fn flow_lines(&self, id: NodeId, spacing: Vec2, limit: Option<f32>) -> (Vec<Vec<NodeId>>, Vec2) {
        let mut lines: Vec<Vec<NodeId>> = Vec::new();
        let mut line_w = 0.0f32;
        let mut max_w = 0.0f32;
        let mut heights: Vec<f32> = Vec::new();
        for child in self.visible_children(id) {
            let size = self.outer(child);
            let start_new = match (lines.last(), limit) {
                (None, _) => true,
                (Some(_), Some(limit)) => line_w + spacing.x + size.x > limit,
                (Some(_), None) => false,
            };
            if start_new {
                max_w = max_w.max(line_w);
                lines.push(Vec::new());
                heights.push(0.0);
                line_w = size.x;
            } else {
                line_w += spacing.x + size.x;
            }
            lines.last_mut().unwrap().push(child);
            let h = heights.last_mut().unwrap();
            *h = h.max(size.y);
        }
        max_w = max_w.max(line_w);
        let total_h = sum_with_spacing(&heights, spacing.y);
        (lines, Vec2::new(max_w, total_h))
    }

    fn arrange(&mut self, id: NodeId, rect: Rect) {
        self.geometry.insert(id, rect);
        self.nodes[id].bounds.set(rect);
        if let Kind::Text(style) = &self.nodes[id].kind
            && (style.wrap.get() != WrapMode::NoWrap || style.elide.get() != Elide::None)
        {
            let params = text_params(style, Some(rect.w), self.font_generation);
            self.shape_text(id, style, params);
        }
        let children: Vec<NodeId> = self.visible_children(id).collect();
        match self.nodes[id].layout.get() {
            Layout::Free => self.arrange_free(&children, rect),
            Layout::Row { spacing, padding } => self.arrange_linear(&children, inset(rect, padding), spacing, true),
            Layout::Column { spacing, padding } => self.arrange_linear(&children, inset(rect, padding), spacing, false),
            Layout::Grid { columns, spacing, padding } => {
                self.arrange_grid(id, inset(rect, padding), columns, spacing);
            }
            Layout::Flow { spacing, padding } => self.arrange_flow(id, inset(rect, padding), spacing),
            Layout::Stack { padding } => {
                let inner = inset(rect, padding);
                for child in children {
                    let r = self.fit_in(child, inner, true, true, Align::Start, Align::Start);
                    self.arrange(child, r);
                }
            }
        }
    }

    fn arrange_free(&mut self, children: &[NodeId], parent: Rect) {
        let mut pending: Vec<NodeId> = children.to_vec();
        while !pending.is_empty() {
            let ready = pending
                .iter()
                .position(|c| self.nodes[*c].anchors.get().references().all(|r| r == *c || !pending.contains(&r)));
            let child = pending.remove(ready.unwrap_or(0));
            let r = self.place_free(child, parent);
            self.arrange(child, r);
        }
    }

    fn fit_in(&self, id: NodeId, area: Rect, fill_w: bool, fill_h: bool, h_align: Align, v_align: Align) -> Rect {
        let c = self.constraints(id);
        let m = c.margins;
        let avail = Vec2::new((area.w - m.horizontal()).max(0.0), (area.h - m.vertical()).max(0.0));
        let p = self.preferred(id);
        let w = if fill_w { c.clamp_width(avail.x) } else { p.x };
        let h = if fill_h { c.clamp_height(avail.y) } else { p.y };
        let x = area.x + m.left + h_align.offset(avail.x - w);
        let y = area.y + m.top + v_align.offset(avail.y - h);
        Rect::new(x, y, w, h)
    }

    fn arrange_grid(&mut self, id: NodeId, inner: Rect, columns: usize, spacing: Vec2) {
        let metrics = self.grid_metrics(id, columns, spacing);
        let GridMetrics { cells, mut columns, mut rows } = metrics;
        let fill_columns: Vec<usize> = (0..columns.len())
            .filter(|&k| {
                cells
                    .iter()
                    .any(|c| c.column <= k && k < c.column + c.column_span && self.nodes[c.child].fill_width.get())
            })
            .collect();
        let fill_rows: Vec<usize> = (0..rows.len())
            .filter(|&k| {
                cells.iter().any(|c| c.row <= k && k < c.row + c.row_span && self.nodes[c.child].fill_height.get())
            })
            .collect();
        let extra_w = inner.w - sum_with_spacing(&columns, spacing.x);
        if extra_w > 0.0 && !fill_columns.is_empty() {
            let share = extra_w / fill_columns.len() as f32;
            fill_columns.iter().for_each(|&k| columns[k] += share);
        }
        let extra_h = inner.h - sum_with_spacing(&rows, spacing.y);
        if extra_h > 0.0 && !fill_rows.is_empty() {
            let share = extra_h / fill_rows.len() as f32;
            fill_rows.iter().for_each(|&k| rows[k] += share);
        }
        let column_x = offsets(&columns, spacing.x, inner.x);
        let row_y = offsets(&rows, spacing.y, inner.y);
        for cell in cells {
            let w = sum_with_spacing(&columns[cell.column..cell.column + cell.column_span], spacing.x);
            let h = sum_with_spacing(&rows[cell.row..cell.row + cell.row_span], spacing.y);
            let area = Rect::new(column_x[cell.column], row_y[cell.row], w, h);
            let node = &self.nodes[cell.child];
            let v_align = node.align.get();
            let r = self.fit_in(cell.child, area, node.fill_width.get(), node.fill_height.get(), Align::Start, v_align);
            self.arrange(cell.child, r);
        }
    }

    fn arrange_flow(&mut self, id: NodeId, inner: Rect, spacing: Vec2) {
        let (lines, _) = self.flow_lines(id, spacing, Some(inner.w));
        let mut y = inner.y;
        for line in lines {
            let height = line.iter().map(|c| self.outer(*c).y).fold(0.0, f32::max);
            let mut x = inner.x;
            for child in line {
                let size = self.outer(child);
                let area = Rect::new(x, y, size.x, height);
                let align = self.nodes[child].align.get();
                let r = self.fit_in(child, area, false, false, Align::Start, align);
                self.arrange(child, r);
                x += size.x + spacing.x;
            }
            y += height + spacing.y;
        }
    }

    fn arrange_linear(&mut self, children: &[NodeId], inner: Rect, spacing: f32, horizontal: bool) {
        let fills = |n: &Node| if horizontal { n.fill_width.get() } else { n.fill_height.get() };
        let cross_fills = |n: &Node| if horizontal { n.fill_height.get() } else { n.fill_width.get() };
        let (main_avail, cross_avail) = if horizontal { (inner.w, inner.h) } else { (inner.h, inner.w) };
        let split = |v: Vec2| if horizontal { (v.x, v.y) } else { (v.y, v.x) };
        let margin_main = |m: Margins| if horizontal { m.horizontal() } else { m.vertical() };
        let margin_cross = |m: Margins| if horizontal { m.vertical() } else { m.horizontal() };
        let margin_start = |m: Margins| if horizontal { (m.left, m.top) } else { (m.top, m.left) };
        let limits = |c: &Constraints| {
            if horizontal { (c.min_width, c.max_width) } else { (c.min_height, c.max_height) }
        };

        let mut sizes: Vec<f32> = Vec::with_capacity(children.len());
        let mut active: Vec<usize> = Vec::new();
        let mut remaining = main_avail - spacing * (children.len().max(1) - 1) as f32;
        for (i, &c) in children.iter().enumerate() {
            let node = &self.nodes[c];
            let cons = node.constraints.get();
            remaining -= margin_main(cons.margins);
            if fills(node) {
                sizes.push(0.0);
                active.push(i);
            } else {
                let main = split(self.preferred(c)).0;
                sizes.push(main);
                remaining -= main;
            }
        }
        while !active.is_empty() {
            let share = (remaining / active.len() as f32).max(0.0);
            let mut frozen = false;
            let mut still = Vec::new();
            for &i in &active {
                let (lo, hi) = limits(&self.constraints(children[i]));
                if share < lo || share > hi {
                    let value = share.min(hi).max(lo);
                    sizes[i] = value;
                    remaining -= value;
                    frozen = true;
                } else {
                    still.push(i);
                }
            }
            if !frozen {
                still.iter().for_each(|&i| sizes[i] = share);
                break;
            }
            active = still;
        }

        let mut cursor = 0.0;
        for (i, &c) in children.iter().enumerate() {
            let node = &self.nodes[c];
            let cons = node.constraints.get();
            let m = cons.margins;
            let main = sizes[i];
            let cross_limits =
                if horizontal { (cons.min_height, cons.max_height) } else { (cons.min_width, cons.max_width) };
            let cross = if cross_fills(node) {
                (cross_avail - margin_cross(m)).max(0.0).min(cross_limits.1).max(cross_limits.0)
            } else {
                split(self.preferred(c)).1
            };
            let (start_main, start_cross) = margin_start(m);
            let offset = start_cross + node.align.get().offset(cross_avail - margin_cross(m) - cross);
            let main_pos = cursor + start_main;
            let r = if horizontal {
                Rect::new(inner.x + main_pos, inner.y + offset, main, cross)
            } else {
                Rect::new(inner.x + offset, inner.y + main_pos, cross, main)
            };
            cursor += main + margin_main(m) + spacing;
            self.arrange(c, r);
        }
    }

    fn edge_coord(&self, target: Option<(NodeId, Edge)>) -> Option<f32> {
        let (id, edge) = target?;
        let r = self.geometry.get(id)?;
        Some(match edge {
            Edge::Left => r.x,
            Edge::Right => r.x + r.w,
            Edge::Top => r.y,
            Edge::Bottom => r.y + r.h,
            Edge::HCenter => r.x + r.w / 2.0,
            Edge::VCenter => r.y + r.h / 2.0,
        })
    }

    fn place_free(&self, id: NodeId, parent: Rect) -> Rect {
        let node = &self.nodes[id];
        let a = node.anchors.get();
        let p = self.preferred(id);
        let near = |margin: Option<f32>, target: Option<(NodeId, Edge)>, base: f32| {
            let reference = self.edge_coord(target);
            match (margin, reference) {
                (None, None) => None,
                (m, r) => Some(r.unwrap_or(base) + m.unwrap_or(0.0)),
            }
        };
        let far = |margin: Option<f32>, target: Option<(NodeId, Edge)>, base: f32| {
            let reference = self.edge_coord(target);
            match (margin, reference) {
                (None, None) => None,
                (m, r) => Some(r.unwrap_or(base) - m.unwrap_or(0.0)),
            }
        };
        let (x, w) = resolve_axis(
            parent.x,
            parent.w,
            p.x,
            node.x.get(),
            near(a.left, a.left_ref, parent.x),
            far(a.right, a.right_ref, parent.x + parent.w),
            a.h_center.then_some(a.h_offset),
        );
        let (y, h) = resolve_axis(
            parent.y,
            parent.h,
            p.y,
            node.y.get(),
            near(a.top, a.top_ref, parent.y),
            far(a.bottom, a.bottom_ref, parent.y + parent.h),
            a.v_center.then_some(a.v_offset),
        );
        let c = self.constraints(id);
        Rect::new(x, y, c.clamp_width(w), c.clamp_height(h))
    }
}

fn sum_with_spacing(values: &[f32], spacing: f32) -> f32 {
    values.iter().sum::<f32>() + spacing * (values.len().max(1) - 1) as f32
}

fn offsets(sizes: &[f32], spacing: f32, start: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(sizes.len());
    let mut cursor = start;
    for size in sizes {
        out.push(cursor);
        cursor += size + spacing;
    }
    out
}

fn resolve_axis(
    start: f32,
    extent: f32,
    size: f32,
    offset: f32,
    near: Option<f32>,
    far: Option<f32>,
    center: Option<f32>,
) -> (f32, f32) {
    match (near, far, center) {
        (Some(n), Some(f), _) => (n, (f - n).max(0.0)),
        (Some(n), None, _) => (n, size),
        (None, Some(f), _) => (f - size, size),
        (None, None, Some(c)) => (start + ((extent - size) / 2.0).round() + c, size),
        (None, None, None) => (start + offset, size),
    }
}

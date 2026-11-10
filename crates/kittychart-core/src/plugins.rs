//! Built-in plugins with layout boxes: title, subtitle and legend.

use kittychart_canvas::{Canvas, Color, LineCap, LineJoin, Paint, TextAlign, TextBaseline};

use crate::config::Label;
use crate::draw::{PointStyle, TextOpts, clip_area, draw_point, render_text, rounded_rect_path};
use crate::layout::LayoutItem;
use crate::options::{Corners, FontSpec, Padding};
use crate::scale::Position;

#[derive(Debug, Clone, Copy, Default)]
pub struct BoxGeom {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

impl BoxGeom {
    fn set(&mut self, left: f64, top: f64, width: f64, height: f64) {
        *self = BoxGeom { left, top, right: left + width, bottom: top + height, width, height };
    }
}

// ------------------------------------------------------------------ title / subtitle

/// The `title` and `subtitle` plugins (same element, different defaults).
#[derive(Debug, Clone)]
pub struct TextBlock {
    pub display: bool,
    pub text: Label,
    pub font: FontSpec,
    pub color: Option<Color>,
    pub padding: Padding,
    pub align: String,
    pub position: Position,
    pub weight: f64,
    pub full_size: bool,
    pub geom: BoxGeom,
}

impl TextBlock {
    pub fn draw(&self, c: &mut dyn Canvas) {
        if !self.display {
            return;
        }
        let lh = self.font.line_height;
        let offset = lh / 2.0 + self.padding.top;
        let g = self.geom;
        let (x, y, max_width, rotation) = if self.position.is_horizontal() {
            (align_start_end(&self.align, g.left, g.right), g.top + offset, g.right - g.left, 0.0)
        } else if self.position == Position::Left {
            (
                g.left + offset,
                align_start_end(&self.align, g.bottom, g.top),
                g.bottom - g.top,
                -std::f64::consts::FRAC_PI_2,
            )
        } else {
            (
                g.right - offset,
                align_start_end(&self.align, g.top, g.bottom),
                g.bottom - g.top,
                std::f64::consts::FRAC_PI_2,
            )
        };
        render_text(
            c,
            &self.text,
            0.0,
            0.0,
            &self.font,
            &TextOpts {
                color: self.color,
                max_width: Some(max_width),
                rotation: Some(rotation),
                align: Some(to_left_right_center(&self.align)),
                baseline: Some(TextBaseline::Middle),
                translation: Some((x, y)),
                ..TextOpts::default()
            },
        );
    }
}

impl LayoutItem for TextBlock {
    fn position(&self) -> Position {
        self.position
    }
    fn weight(&self) -> f64 {
        self.weight
    }
    fn full_size(&self) -> bool {
        self.full_size
    }
    fn is_horizontal(&self) -> bool {
        self.position.is_horizontal()
    }
    fn displayed(&self) -> bool {
        self.display
    }
    fn update(&mut self, max_width: f64, max_height: f64, _m: Padding, _c: &mut dyn Canvas) {
        self.geom.left = 0.0;
        self.geom.top = 0.0;
        if !self.display {
            self.geom = BoxGeom::default();
            return;
        }
        self.geom.width = max_width;
        self.geom.right = max_width;
        self.geom.height = max_height;
        self.geom.bottom = max_height;
        let size = self.text.line_count() as f64 * self.font.line_height + self.padding.height();
        if self.is_horizontal() {
            self.geom.height = size;
        } else {
            self.geom.width = size;
        }
    }
    fn width(&self) -> f64 {
        self.geom.width
    }
    fn height(&self) -> f64 {
        self.geom.height
    }
    fn set_box(&mut self, left: f64, top: f64, width: f64, height: f64) {
        self.geom.set(left, top, width, height);
    }
    fn right(&self) -> f64 {
        self.geom.right
    }
    fn bottom(&self) -> f64 {
        self.geom.bottom
    }
}

// ------------------------------------------------------------------ legend

#[derive(Debug, Clone)]
pub struct LegendItem {
    pub text: Label,
    pub fill_style: Option<Color>,
    pub stroke_style: Option<Color>,
    pub line_width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub line_dash: Vec<f64>,
    pub line_dash_offset: f64,
    pub point_style: PointStyle,
    pub rotation: f64,
    pub hidden: bool,
    pub border_radius: Option<Corners>,
    pub dataset_index: usize,
}

#[derive(Debug, Clone)]
pub struct LegendLabels {
    pub box_width: f64,
    pub box_height: Option<f64>,
    pub padding: f64,
    pub font: FontSpec,
    pub color: Option<Color>,
    pub use_point_style: bool,
    pub point_style_width: Option<f64>,
    pub text_align: TextAlign,
}

#[derive(Debug, Clone)]
pub struct LegendTitle {
    pub display: bool,
    pub text: Label,
    pub font: FontSpec,
    pub color: Option<Color>,
    pub padding: Padding,
    pub position: String,
}

/// Item hit boxes from layout. Unused while charts are static. v2 legend clicks/hover read them.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct HitBox {
    left: f64,
    top: f64,
    row_or_col: usize,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
pub struct Legend {
    pub display: bool,
    pub position: Position,
    pub align: String,
    pub weight: f64,
    pub full_size: bool,
    pub max_width_opt: Option<f64>,
    pub max_height_opt: Option<f64>,
    pub labels: LegendLabels,
    pub title: LegendTitle,
    pub items: Vec<LegendItem>,
    pub geom: BoxGeom,
    max_width: f64,
    max_height: f64,
    margins: Padding,
    line_widths: Vec<f64>,
    column_sizes: Vec<(f64, f64)>,
    hit_boxes: Vec<HitBox>,
}

impl Legend {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        display: bool,
        position: Position,
        align: String,
        weight: f64,
        full_size: bool,
        max_width_opt: Option<f64>,
        max_height_opt: Option<f64>,
        labels: LegendLabels,
        title: LegendTitle,
        items: Vec<LegendItem>,
    ) -> Self {
        Legend {
            display,
            position,
            align,
            weight,
            full_size,
            max_width_opt,
            max_height_opt,
            labels,
            title,
            items,
            geom: BoxGeom::default(),
            max_width: 0.0,
            max_height: 0.0,
            margins: Padding::default(),
            line_widths: vec![],
            column_sizes: vec![],
            hit_boxes: vec![],
        }
    }

    fn box_size(&self) -> (f64, f64, f64) {
        let font_size = self.labels.font.size;
        let mut box_height = self.labels.box_height.unwrap_or(font_size);
        let mut box_width = self.labels.box_width;
        if self.labels.use_point_style {
            box_height = box_height.min(font_size);
            box_width = self.labels.point_style_width.unwrap_or(box_width.min(font_size));
        }
        (box_width, box_height, font_size.max(box_height))
    }

    fn title_height(&self) -> f64 {
        if self.title.display { self.title.font.line_height + self.title.padding.height() } else { 0.0 }
    }

    fn item_width(&self, item: &LegendItem, box_width: f64, c: &mut dyn Canvas) -> f64 {
        let longest = item.text.lines().into_iter().max_by_key(|l| l.len()).unwrap_or("").to_owned();
        box_width + self.labels.font.size / 2.0 + f64::from(c.measure_text(&longest).width)
    }

    fn fit_rows(
        &mut self,
        title_height: f64,
        font_size: f64,
        box_width: f64,
        item_height: f64,
        c: &mut dyn Canvas,
    ) -> f64 {
        let padding = self.labels.padding;
        let line_height = item_height + padding;
        let mut total = title_height;
        self.line_widths = vec![0.0];
        self.hit_boxes.clear();
        let mut row: isize = -1;
        let mut top = -line_height;
        for i in 0..self.items.len() {
            let text = self.items[i].text.lines().join(" ");
            let item_width = box_width + font_size / 2.0 + f64::from(c.measure_text(&text).width);
            let last = *self.line_widths.last().unwrap_or(&0.0);
            if i == 0 || last + item_width + 2.0 * padding > self.max_width {
                total += line_height;
                if i > 0 {
                    self.line_widths.push(0.0);
                } else {
                    *self.line_widths.last_mut().expect("non-empty") = 0.0;
                }
                top += line_height;
                row += 1;
            }
            self.hit_boxes.push(HitBox {
                left: 0.0,
                top,
                row_or_col: row.max(0) as usize,
                width: item_width,
                height: item_height,
            });
            *self.line_widths.last_mut().expect("non-empty") += item_width + padding;
        }
        total
    }

    fn fit_cols(
        &mut self,
        title_height: f64,
        box_width: f64,
        item_height_base: f64,
        c: &mut dyn Canvas,
    ) -> f64 {
        let padding = self.labels.padding;
        let height_limit = self.max_height - title_height;
        let mut total_width = padding;
        let (mut col_w, mut col_h, mut left) = (0.0f64, 0.0f64, 0.0f64);
        let mut col = 0;
        self.column_sizes.clear();
        self.hit_boxes.clear();
        for i in 0..self.items.len() {
            let item = self.items[i].clone();
            let item_width = self.item_width(&item, box_width, c);
            let item_height = if item.text.line_count() > 1 {
                self.labels.font.line_height * item.text.line_count() as f64
            } else {
                item_height_base
            };
            if i > 0 && col_h + item_height + 2.0 * padding > height_limit {
                total_width += col_w + padding;
                self.column_sizes.push((col_w, col_h));
                left += col_w + padding;
                col += 1;
                col_w = 0.0;
                col_h = 0.0;
            }
            self.hit_boxes.push(HitBox {
                left,
                top: col_h,
                row_or_col: col,
                width: item_width,
                height: item_height,
            });
            col_w = col_w.max(item_width);
            col_h += item_height + padding;
        }
        total_width += col_w;
        self.column_sizes.push((col_w, col_h));
        total_width
    }

    pub fn draw(&self, c: &mut dyn Canvas) {
        if !self.display {
            return;
        }
        let g = self.geom;
        clip_area(c, g.left, g.top, g.right, g.bottom);
        self.draw_inner(c);
        c.restore();
    }

    fn draw_inner(&self, c: &mut dyn Canvas) {
        let font = &self.labels.font;
        let padding = self.labels.padding;
        let font_size = font.size;
        let half = font_size / 2.0;
        let (box_width, box_height, item_height) = self.box_size();
        let title_height = self.title_height();
        let g = self.geom;
        self.draw_title(c);

        c.save();
        c.set_font(font.to_canvas());
        c.set_text_align(TextAlign::Left);
        c.set_text_baseline(TextBaseline::Middle);
        c.set_line_width(0.5);

        let horizontal = self.position.is_horizontal();
        let line_height = item_height + padding;
        let mut line = 0;
        let (mut cx, mut cy) = if horizontal {
            (
                align_start_end(
                    &self.align,
                    g.left + padding,
                    g.right - self.line_widths.first().copied().unwrap_or(0.0),
                ),
                g.top + padding + title_height,
            )
        } else {
            (
                g.left + padding,
                align_start_end(
                    &self.align,
                    g.top + title_height + padding,
                    g.bottom - self.column_sizes.first().map_or(0.0, |s| s.1),
                ),
            )
        };
        let text_color = self.labels.color.unwrap_or(Color::rgba(0.4, 0.4, 0.4, 1.0));
        for (i, item) in self.items.iter().enumerate() {
            c.set_font(font.to_canvas());
            let text_width =
                item.text.lines().iter().map(|l| f64::from(c.measure_text(l).width)).fold(0.0, f64::max);
            let width = box_width + half + text_width;
            let (mut x, mut y) = (cx, cy);
            if horizontal {
                if i > 0 && x + width + padding > g.right {
                    cy += line_height;
                    y = cy;
                    line += 1;
                    cx = align_start_end(
                        &self.align,
                        g.left + padding,
                        g.right - self.line_widths.get(line).copied().unwrap_or(0.0),
                    );
                    x = cx;
                }
            } else if i > 0 && y + line_height > g.bottom {
                cx = x + self.column_sizes.get(line).map_or(0.0, |s| s.0) + padding;
                x = cx;
                line += 1;
                cy = align_start_end(
                    &self.align,
                    g.top + title_height + padding,
                    g.bottom - self.column_sizes.get(line).map_or(0.0, |s| s.1),
                );
                y = cy;
            }

            self.draw_box(c, x, y, item, box_width, box_height, half);

            let text_x = text_x(
                self.labels.text_align,
                x + box_width + half,
                if horizontal { x + width } else { g.right },
            );
            render_text(
                c,
                &item.text,
                text_x,
                y + item_height / 2.0,
                font,
                &TextOpts {
                    color: Some(text_color),
                    align: Some(self.labels.text_align),
                    baseline: Some(TextBaseline::Middle),
                    strikethrough: item.hidden,
                    ..TextOpts::default()
                },
            );
            if horizontal {
                cx += width + padding;
            } else if item.text.line_count() > 1 {
                cy += font.line_height * item.text.line_count() as f64 + padding;
            } else {
                cy += line_height;
            }
        }
        c.restore();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_box(
        &self,
        c: &mut dyn Canvas,
        x: f64,
        y: f64,
        item: &LegendItem,
        box_width: f64,
        box_height: f64,
        half: f64,
    ) {
        if box_width.is_nan() || box_width <= 0.0 || box_height.is_nan() || box_height < 0.0 {
            return;
        }
        let default = Color::rgba(0.4, 0.4, 0.4, 1.0);
        c.save();
        c.set_fill_style(Paint::Solid(item.fill_style.unwrap_or(default)));
        c.set_line_cap(item.line_cap);
        c.set_line_dash_offset(item.line_dash_offset as f32);
        c.set_line_join(item.line_join);
        c.set_line_width(item.line_width as f32);
        c.set_stroke_style(Paint::Solid(item.stroke_style.unwrap_or(default)));
        let dash: Vec<f32> = item.line_dash.iter().map(|d| *d as f32).collect();
        c.set_line_dash(&dash);
        if self.labels.use_point_style {
            let radius = box_height * std::f64::consts::SQRT_2 / 2.0;
            let w = self.labels.point_style_width.map(|_| box_width);
            draw_point(
                c,
                item.point_style,
                radius,
                item.rotation,
                item.line_width,
                x + box_width / 2.0,
                y + half,
                w,
            );
        } else {
            let y_top = y + ((self.labels.font.size - box_height) / 2.0).max(0.0);
            c.begin_path();
            match item.border_radius {
                Some(r) if r.any() => rounded_rect_path(c, x, y_top, box_width, box_height, r),
                _ => c.rect(x as f32, y_top as f32, box_width as f32, box_height as f32),
            }
            c.fill();
            if item.line_width != 0.0 {
                c.stroke();
            }
        }
        c.restore();
    }

    fn draw_title(&self, c: &mut dyn Canvas) {
        let t = &self.title;
        if !t.display {
            return;
        }
        let g = self.geom;
        let half = t.font.size / 2.0;
        let top_pad_half = t.padding.top + half;
        let mut left = g.left;
        let mut max_width = g.width;
        let y;
        if self.position.is_horizontal() {
            max_width = self.line_widths.iter().copied().fold(0.0, f64::max);
            y = g.top + top_pad_half;
            left = align_start_end(&self.align, left, g.right - max_width);
        } else {
            let max_height = self.column_sizes.iter().map(|s| s.1).fold(0.0, f64::max);
            y = top_pad_half
                + align_start_end(
                    &self.align,
                    g.top,
                    g.bottom - max_height - self.labels.padding - self.title_height(),
                );
        }
        let x = align_start_end(&t.position, left, left + max_width);
        render_text(
            c,
            &t.text,
            x,
            y,
            &t.font,
            &TextOpts {
                color: t.color,
                align: Some(to_left_right_center(&t.position)),
                baseline: Some(TextBaseline::Middle),
                ..TextOpts::default()
            },
        );
    }
}

impl LayoutItem for Legend {
    fn position(&self) -> Position {
        self.position
    }
    fn weight(&self) -> f64 {
        self.weight
    }
    fn full_size(&self) -> bool {
        self.full_size
    }
    fn is_horizontal(&self) -> bool {
        self.position.is_horizontal()
    }
    fn displayed(&self) -> bool {
        self.display
    }
    fn update(&mut self, max_width: f64, max_height: f64, margins: Padding, c: &mut dyn Canvas) {
        self.max_width = max_width;
        self.max_height = max_height;
        self.margins = margins;
        // setDimensions
        if self.is_horizontal() {
            self.geom.width = max_width;
            self.geom.left = margins.left;
            self.geom.right = max_width;
        } else {
            self.geom.height = max_height;
            self.geom.top = margins.top;
            self.geom.bottom = max_height;
        }
        // fit
        if !self.display {
            self.geom.width = 0.0;
            self.geom.height = 0.0;
            return;
        }
        c.save();
        c.set_font(self.labels.font.to_canvas());
        let title_height = self.title_height();
        let (box_width, _, item_height) = self.box_size();
        let (width, height) = if self.is_horizontal() {
            (max_width, self.fit_rows(title_height, self.labels.font.size, box_width, item_height, c) + 10.0)
        } else {
            (self.fit_cols(title_height, box_width, item_height, c) + 10.0, max_height)
        };
        c.restore();
        self.geom.width = width.min(self.max_width_opt.unwrap_or(max_width));
        self.geom.height = height.min(self.max_height_opt.unwrap_or(max_height));
    }
    fn width(&self) -> f64 {
        self.geom.width
    }
    fn height(&self) -> f64 {
        self.geom.height
    }
    fn set_box(&mut self, left: f64, top: f64, width: f64, height: f64) {
        self.geom.set(left, top, width, height);
    }
    fn right(&self) -> f64 {
        self.geom.right
    }
    fn bottom(&self) -> f64 {
        self.geom.bottom
    }
}

pub fn align_start_end(align: &str, start: f64, end: f64) -> f64 {
    match align {
        "start" => start,
        "end" => end,
        _ => (start + end) / 2.0,
    }
}

pub fn to_left_right_center(align: &str) -> TextAlign {
    match align {
        "start" => TextAlign::Left,
        "end" => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

fn text_x(align: TextAlign, left: f64, right: f64) -> f64 {
    match align {
        TextAlign::Right | TextAlign::End => right,
        TextAlign::Center => (left + right) / 2.0,
        _ => left,
    }
}

use cosmic::{
    Element, Renderer, Theme,
    cosmic_theme::palette::WithAlpha,
    iced::{Color, Length, Point, Rectangle, mouse},
    widget::{canvas, column, row, space, text},
};
use std::{collections::VecDeque, time::Instant};

use super::{
    Message, fl,
    info::{GpuId, GraphItem},
};

pub const SMALL_GRAPH_HEIGHT: f32 = 207.0;
const LARGE_GRAPH_HEIGHT: f32 = 280.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcGraphKind {
    #[default]
    Utilization,
    Frequency,
    Power,
    Temperature,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Unit {
    Percent,
    Frequency,
    Power,
    Temperature,
    Throughput,
}

impl From<ProcGraphKind> for Unit {
    fn from(kind: ProcGraphKind) -> Self {
        match kind {
            ProcGraphKind::Utilization => Unit::Percent,
            ProcGraphKind::Frequency => Unit::Frequency,
            ProcGraphKind::Power => Unit::Power,
            ProcGraphKind::Temperature => Unit::Temperature,
        }
    }
}

impl Unit {
    fn title(self) -> String {
        match self {
            Unit::Percent => fl!("utilization-graph"),
            Unit::Frequency => fl!("frequency-graph"),
            Unit::Power => fl!("power-graph"),
            Unit::Temperature => fl!("temperature-graph"),
            Unit::Throughput => fl!("throughput-graph"),
        }
    }

    fn format(self, value: f32) -> String {
        match self {
            Unit::Percent => format!("{:.0}%", value),
            Unit::Frequency => {
                if value >= 1000.0 {
                    format!("{:.0} GHz", value / 1000.0)
                } else {
                    format!("{:.0} MHz", value)
                }
            }
            Unit::Power => format!("{:.0} W", value),
            Unit::Temperature => format!("{:.0}°C", value),
            Unit::Throughput => {
                let format_options =
                    humansize::FormatSizeOptions::from(humansize::DECIMAL).decimal_places(0);
                format!("{}/s", humansize::format_size(value as u64, format_options))
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum GraphKind<'a> {
    Cpu(ProcGraphKind),
    Memory,
    Swap,
    Gpu(GpuId, ProcGraphKind),
    GpuVram(GpuId),
    DiskRead(&'a str),
    DiskWrite(&'a str),
    DiskTotal,
    NetworkRx(&'a str),
    NetworkTx(&'a str),
    NetworkTotal,
}

impl<'a> GraphKind<'a> {
    fn unit(self) -> Unit {
        match self {
            GraphKind::Cpu(k) | GraphKind::Gpu(_, k) => k.into(),
            GraphKind::Memory | GraphKind::Swap | GraphKind::GpuVram(_) => Unit::Percent,
            GraphKind::DiskRead(_)
            | GraphKind::DiskWrite(_)
            | GraphKind::DiskTotal
            | GraphKind::NetworkRx(_)
            | GraphKind::NetworkTx(_)
            | GraphKind::NetworkTotal => Unit::Throughput,
        }
    }
}

pub struct Graph<'a> {
    pub kind: GraphKind<'a>,
    pub history: &'a VecDeque<GraphItem>,
    pub small: bool,
}

impl<'a> Graph<'a> {
    pub fn new(kind: GraphKind<'a>, history: &'a VecDeque<GraphItem>) -> Self {
        Self {
            kind,
            history,
            small: false,
        }
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    fn scale_y(&self) -> f32 {
        match self.kind.unit() {
            Unit::Percent => 100.0,
            _ => {
                let max = self
                    .history
                    .iter()
                    .map(|graph_item| graph_item.value(self.kind))
                    .fold(0.0, f32::max);
                10.0f32.powf(max.log10().ceil().max(2.0))
            }
        }
    }
}

impl<'a> From<Graph<'a>> for Element<'a, Message> {
    fn from(graph: Graph<'a>) -> Self {
        let (height, legend) = if graph.small {
            (SMALL_GRAPH_HEIGHT, None)
        } else {
            (
                LARGE_GRAPH_HEIGHT,
                Some(row!(
                    text::body(graph.kind.unit().title()),
                    space::horizontal(),
                    text::body(graph.kind.unit().format(graph.scale_y()))
                )),
            )
        };

        column::with_capacity(2)
            .push_maybe(legend)
            .push(canvas(graph).height(height).width(Length::Fill))
            .into()
    }
}

impl<'a> canvas::Program<Message, Theme, Renderer> for Graph<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let cosmic = theme.cosmic();
        let accent_color = Color::from(cosmic.accent_color());
        let accent_color_0_5 = Color::from(cosmic.accent_color().with_alpha(0.5));
        let bg_component_color = Color::from(cosmic.bg_component_color());
        let bg_component_divider = Color::from(cosmic.bg_component_divider());
        //TODO: design has radius_s but Canvas does not support clipping with border radius
        //let bg_radius = cosmic.radius_s();
        let bg_radius = cosmic.radius_0();

        // NaN or +/- infinity would be nonsensical on a chart, so replace with zero
        fn invalid_is_zero(value: f32) -> f32 {
            if value.is_finite() { value } else { 0.0 }
        }
        let scale_y = self.scale_y();
        let calc_x = |time: f32| -> f32 { (1.0 - time / 60.0) * bounds.width };
        let calc_y =
            |value: f32| -> f32 { (1.0 - invalid_is_zero(value / scale_y)) * bounds.height };

        let min_x = calc_x(60.0);
        let max_x = calc_x(0.0);
        let min_y = calc_y(scale_y);
        let max_y = calc_y(0.0);

        //TODO: use cache
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Draw background
        let path = canvas::Path::rounded_rectangle(Point::ORIGIN, bounds.size(), bg_radius.into());
        frame.fill(&path, bg_component_color);
        // Draw border
        if self.small {
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(bg_component_divider),
            );
        }

        // Draw grid lines
        let grid = canvas::Path::new(|b| {
            for t in [50.0, 40.0, 30.0, 20.0, 10.0] {
                let x = calc_x(t);
                b.move_to(Point::new(x, min_y));
                b.line_to(Point::new(x, max_y));
            }
            for v in [scale_y * 0.2, scale_y * 0.4, scale_y * 0.6, scale_y * 0.8] {
                let y = calc_y(v);
                b.move_to(Point::new(min_x, y));
                b.line_to(Point::new(max_x, y));
            }
        });
        frame.stroke(
            &grid,
            canvas::Stroke::default().with_color(accent_color_0_5),
        );

        // Draw values
        let start = self
            .history
            .front()
            .map(|x| x.time)
            .unwrap_or_else(Instant::now);
        let end = self
            .history
            .back()
            .map(|x| x.time)
            .unwrap_or_else(Instant::now);
        let mut area = canvas::path::Builder::new();
        let mut line = canvas::path::Builder::new();
        area.move_to(Point::new(
            calc_x(end.saturating_duration_since(start).as_secs_f32()),
            max_y,
        ));
        for (i, graph_item) in self.history.iter().enumerate() {
            let x = calc_x(end.saturating_duration_since(graph_item.time).as_secs_f32());
            let y = calc_y(graph_item.value(self.kind));
            let point = Point::new(x, y);
            area.line_to(point);
            if i == 0 {
                line.move_to(point)
            } else {
                line.line_to(point);
            }
        }
        area.line_to(Point::new(max_x, max_y));
        area.close();
        frame.fill(&area.build(), accent_color_0_5);
        frame.stroke(
            &line.build(),
            canvas::Stroke::default().with_color(accent_color),
        );

        vec![frame.into_geometry()]
    }
}

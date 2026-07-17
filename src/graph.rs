use cosmic::{
    Renderer, Theme,
    iced::{Color, Point, Rectangle, Size, alignment::Vertical, core::text::Alignment, mouse},
    widget::canvas,
};
use std::{collections::VecDeque, time::Instant};

use super::{
    Message,
    info::{GpuId, GraphItem},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcGraphKind {
    #[default]
    Utilization,
    Frequency,
    Power,
    Temperature,
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
    fn label(&self, value: f32) -> String {
        match self {
            GraphKind::Cpu(ProcGraphKind::Utilization)
            | GraphKind::Memory
            | GraphKind::Swap
            | GraphKind::Gpu(_, ProcGraphKind::Utilization)
            | GraphKind::GpuVram(_) => {
                format!("{:.0}%", value)
            }
            GraphKind::Cpu(ProcGraphKind::Frequency)
            | GraphKind::Gpu(_, ProcGraphKind::Frequency) => {
                if value >= 1000.0 {
                    format!("{:.0} GHz", value / 1000.0)
                } else {
                    format!("{:.0} MHz", value)
                }
            }
            GraphKind::Cpu(ProcGraphKind::Power) | GraphKind::Gpu(_, ProcGraphKind::Power) => {
                format!("{:.0} W", value)
            }
            GraphKind::Cpu(ProcGraphKind::Temperature)
            | GraphKind::Gpu(_, ProcGraphKind::Temperature) => {
                format!("{:.0}°C", value)
            }
            GraphKind::DiskRead(_)
            | GraphKind::DiskWrite(_)
            | GraphKind::DiskTotal
            | GraphKind::NetworkRx(_)
            | GraphKind::NetworkTx(_)
            | GraphKind::NetworkTotal => {
                let format_options =
                    humansize::FormatSizeOptions::from(humansize::DECIMAL).decimal_places(0);
                format!("{}/s", humansize::format_size(value as u64, format_options))
            }
        }
    }
}

pub struct Graph<'a> {
    pub kind: GraphKind<'a>,
    pub history: &'a VecDeque<GraphItem>,
    pub border: bool,
    pub legend: bool,
}

impl<'a> Graph<'a> {
    pub fn new(kind: GraphKind<'a>, history: &'a VecDeque<GraphItem>) -> Self {
        Self {
            kind,
            history,
            border: false,
            legend: false,
        }
    }

    pub fn border(mut self) -> Self {
        self.border = true;
        self
    }

    pub fn legend(mut self) -> Self {
        self.legend = true;
        self
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
        let mut accent_color_0_5 = accent_color.clone();
        accent_color_0_5.a *= 0.5;
        let bg_component_color = Color::from(cosmic.bg_component_color());
        let bg_component_divider = Color::from(cosmic.bg_component_divider());
        let on_bg_color = Color::from(cosmic.on_bg_color());
        //TODO: design has radius_s but Canvas does not support clipping with border radius
        //let bg_radius = cosmic.radius_s();
        let bg_radius = cosmic.radius_0();

        let (legend_w, legend_h) = if self.legend {
            (80.0, 20.0)
        } else {
            (0.0, 0.0)
        };

        let calc_x = |time: f32| -> f32 { (1.0 - time / 60.0) * (bounds.width - legend_w) };
        let scale_y = match self.kind {
            GraphKind::Cpu(ProcGraphKind::Utilization)
            | GraphKind::Memory
            | GraphKind::Swap
            | GraphKind::Gpu(_, ProcGraphKind::Utilization)
            | GraphKind::GpuVram(_) => 100.0,
            _ => {
                let mut max = 0.0;
                for graph_item in self.history.iter() {
                    max = graph_item.value(self.kind).max(max);
                }
                10.0f32.powf(max.log10().ceil().max(2.0) as f32)
            }
        };

        // NaN or +/- infinity would be nonsensical on a chart, so replace with zero
        fn invalid_is_zero(value: f32) -> f32 {
            if value.is_finite() { value } else { 0.0 }
        }
        let calc_y = |value: f32| -> f32 {
            (1.0 - invalid_is_zero(value / scale_y)) * (bounds.height - legend_h)
        };

        let min_x = calc_x(60.0);
        let max_x = calc_x(0.0);
        let min_y = calc_y(scale_y);
        let max_y = calc_y(0.0);

        //TODO: use cache
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let text = |string: &str,
                    position: Point,
                    align_x: Alignment,
                    align_y: Vertical,
                    frame: &mut canvas::Frame| {
            if self.legend {
                let mut text = canvas::Text::from(string);
                text.position = position;
                text.color = on_bg_color;
                text.align_x = align_x;
                text.align_y = align_y;
                frame.fill_text(text);
            }
        };

        // Draw background
        {
            let path = canvas::Path::rounded_rectangle(
                Point::new(min_x, min_y),
                Size::new(max_x - min_x, max_y - min_y),
                bg_radius.into(),
            );
            frame.fill(&path, bg_component_color);
            if self.border {
                frame.stroke(
                    &path,
                    canvas::Stroke::default().with_color(bg_component_divider),
                );
            }
        }

        // Draw X axis info
        text(
            "60 secs",
            Point::new(calc_x(60.0), max_y),
            Alignment::Left,
            Vertical::Top,
            &mut frame,
        );
        for &(time, string) in &[
            (50.0, "50"),
            (40.0, "40"),
            (30.0, "30"),
            (20.0, "20"),
            (10.0, "10"),
        ] {
            let x = calc_x(time);
            let path = canvas::Path::line(Point::new(x, min_y), Point::new(x, max_y));
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(accent_color_0_5),
            );

            text(
                string,
                Point::new(x, max_y),
                Alignment::Center,
                Vertical::Top,
                &mut frame,
            );
        }
        text(
            "0",
            Point::new(calc_x(0.0), max_y),
            Alignment::Right,
            Vertical::Top,
            &mut frame,
        );

        // Draw Y axis info
        text(
            &self.kind.label(0.0),
            Point::new(max_x, calc_y(0.0)),
            Alignment::Left,
            Vertical::Bottom,
            &mut frame,
        );
        for &value in &[scale_y * 0.2, scale_y * 0.4, scale_y * 0.6, scale_y * 0.8] {
            let y = calc_y(value);
            let path = canvas::Path::line(Point::new(min_x, y), Point::new(max_x, y));
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(accent_color_0_5),
            );

            text(
                &self.kind.label(value),
                Point::new(max_x, y),
                Alignment::Left,
                Vertical::Center,
                &mut frame,
            );
        }
        text(
            &self.kind.label(scale_y),
            Point::new(max_x, calc_y(scale_y)),
            Alignment::Left,
            Vertical::Top,
            &mut frame,
        );

        // Draw values
        let start = self
            .history
            .front()
            .map(|x| x.time)
            .unwrap_or_else(|| Instant::now());
        let end = self
            .history
            .back()
            .map(|x| x.time)
            .unwrap_or_else(|| Instant::now());
        let mut area = canvas::path::Builder::new();
        let mut line = canvas::path::Builder::new();
        area.move_to(Point::new(
            calc_x(end.saturating_duration_since(start).as_secs_f32()),
            calc_y(0.0),
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
        area.line_to(Point::new(calc_x(0.0), calc_y(0.0)));
        area.close();
        frame.fill(&area.build(), accent_color_0_5);
        frame.stroke(
            &line.build(),
            canvas::Stroke::default().with_color(accent_color),
        );

        vec![frame.into_geometry()]
    }
}

use cosmic::{
    Renderer, Theme,
    iced::{Color, Point, Rectangle, Size, mouse},
    widget::canvas,
};
use std::{collections::VecDeque, time::Instant};

use super::{Message, info::GraphItem};

#[derive(Clone, Copy, Debug)]
pub enum GraphKind {
    Cpu,
    Memory,
    Swap,
}

pub struct Graph<'a> {
    pub kind: GraphKind,
    pub history: &'a VecDeque<GraphItem>,
}

impl<'a> Graph<'a> {
    pub fn new(kind: GraphKind, history: &'a VecDeque<GraphItem>) -> Self {
        Self { kind, history }
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
        let radius_s = cosmic.radius_s();

        let calc_x = |time: f32| -> f32 { (1.0 - time / 60.0) * (bounds.width - 36.0) };
        let calc_y = |value: f32| -> f32 { (1.0 - value / 100.0) * (bounds.height - 20.0) };

        let min_x = calc_x(0.0);
        let max_x = calc_x(60.0);
        let min_y = calc_y(0.0);
        let max_y = calc_y(100.0);

        //TODO: use cache
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Draw background
        {
            let path = canvas::Path::rounded_rectangle(
                Point::new(min_x.min(max_x), min_y.min(max_y)),
                Size::new((max_x - min_x).abs(), (max_y - min_y).abs()),
                radius_s.into(),
            );
            frame.fill(&path, bg_component_color)
        }

        // Draw guidelines
        for &time in &[50.0, 40.0, 30.0, 20.0, 10.0] {
            let x = calc_x(time);
            let path = canvas::Path::line(Point::new(x, min_y), Point::new(x, max_y));
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(accent_color_0_5),
            );
        }
        for &value in &[20.0, 40.0, 60.0, 80.0] {
            let y = calc_y(value);
            let path = canvas::Path::line(Point::new(min_x, y), Point::new(max_x, y));
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(accent_color_0_5),
            );
        }

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
            let value = match self.kind {
                GraphKind::Cpu => {
                    graph_item
                        .cpus
                        .iter()
                        .fold(0.0, |total, x| total + x.cpu_usage)
                        / (graph_item.cpus.len() as f32)
                }
                GraphKind::Memory => {
                    100.0 * (graph_item.memory.used as f32) / (graph_item.memory.total as f32)
                }
                GraphKind::Swap => {
                    100.0 * (graph_item.memory.swap_used as f32)
                        / (graph_item.memory.swap_total as f32)
                }
            };
            let y = calc_y(value);
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

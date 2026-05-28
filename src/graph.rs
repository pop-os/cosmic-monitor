use cosmic::{
    Renderer, Theme,
    iced::{Color, Point, Rectangle, Size, alignment::Vertical, core::text::Alignment, mouse},
    widget::canvas,
};
use std::{collections::VecDeque, time::Instant};

use super::{Message, info::GraphItem};

#[derive(Clone, Copy, Debug)]
pub enum GraphKind<'a> {
    Cpu,
    Memory,
    Swap,
    GpuUsage(&'a str),
    GpuVram(&'a str),
    DiskRead(&'a str),
    DiskWrite(&'a str),
    NetworkRx(&'a str),
    NetworkTx(&'a str),
}

pub struct Graph<'a> {
    pub kind: GraphKind<'a>,
    pub history: &'a VecDeque<GraphItem>,
}

impl<'a> Graph<'a> {
    pub fn new(kind: GraphKind<'a>, history: &'a VecDeque<GraphItem>) -> Self {
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
        let on_bg_color = Color::from(cosmic.on_bg_color());
        //TODO: design has radius_s but Canvas does not support clipping with border radius
        //let bg_radius = cosmic.radius_s();
        let bg_radius = cosmic.radius_0();

        let calc_x = |time: f32| -> f32 { (1.0 - time / 60.0) * (bounds.width - 80.0) };
        let scale_y = match self.kind {
            GraphKind::Cpu
            | GraphKind::Memory
            | GraphKind::Swap
            | GraphKind::GpuUsage(_)
            | GraphKind::GpuVram(_) => 100.0,
            GraphKind::DiskRead(disk_name) => {
                let mut max = 0.0;
                for graph_item in self.history.iter() {
                    for disk in graph_item.disks.iter().filter(|x| x.name == disk_name) {
                        max = disk.read.max(max);
                    }
                }
                10.0f32.powf(max.log10().ceil().max(3.0) as f32)
            }
            GraphKind::DiskWrite(disk_name) => {
                let mut max = 0.0;
                for graph_item in self.history.iter() {
                    for disk in graph_item.disks.iter().filter(|x| x.name == disk_name) {
                        max = disk.write.max(max);
                    }
                }
                10.0f32.powf(max.log10().ceil().max(3.0) as f32)
            }
            GraphKind::NetworkRx(network_name) => {
                let mut max = 0.0;
                for graph_item in self.history.iter() {
                    for network in graph_item
                        .networks
                        .iter()
                        .filter(|x| x.name == network_name)
                    {
                        max = network.rx.max(max);
                    }
                }
                10.0f32.powf(max.log10().ceil().max(3.0) as f32)
            }
            GraphKind::NetworkTx(network_name) => {
                let mut max = 0.0;
                for graph_item in self.history.iter() {
                    for network in graph_item
                        .networks
                        .iter()
                        .filter(|x| x.name == network_name)
                    {
                        max = network.tx.max(max);
                    }
                }
                10.0f32.powf(max.log10().ceil().max(3.0) as f32)
            }
        };

        // NaN or +/- infinity would be nonsensical on a chart, so replace with zero
        fn invalid_is_zero(value: f32) -> f32 {
            if value.is_finite() { value } else { 0.0 }
        }
        let calc_y = |value: f32| -> f32 {
            (1.0 - invalid_is_zero(value / scale_y)) * (bounds.height - 20.0)
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
            let mut text = canvas::Text::from(string);
            text.position = position;
            text.color = on_bg_color;
            text.align_x = align_x;
            text.align_y = align_y;
            frame.fill_text(text);
        };

        // Draw background
        {
            let path = canvas::Path::rounded_rectangle(
                Point::new(min_x, min_y),
                Size::new(max_x - min_x, max_y - min_y),
                bg_radius.into(),
            );
            frame.fill(&path, bg_component_color)
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
        match self.kind {
            GraphKind::Cpu
            | GraphKind::Memory
            | GraphKind::Swap
            | GraphKind::GpuUsage(_)
            | GraphKind::GpuVram(_) => {
                text(
                    "0%",
                    Point::new(max_x, calc_y(0.0)),
                    Alignment::Left,
                    Vertical::Bottom,
                    &mut frame,
                );
                for &(value, string) in
                    &[(20.0, "20%"), (40.0, "40%"), (60.0, "60%"), (80.0, "80%")]
                {
                    let y = calc_y(value);
                    let path = canvas::Path::line(Point::new(min_x, y), Point::new(max_x, y));
                    frame.stroke(
                        &path,
                        canvas::Stroke::default().with_color(accent_color_0_5),
                    );

                    text(
                        string,
                        Point::new(max_x, y),
                        Alignment::Left,
                        Vertical::Center,
                        &mut frame,
                    );
                }
                text(
                    "100%",
                    Point::new(max_x, calc_y(100.0)),
                    Alignment::Left,
                    Vertical::Top,
                    &mut frame,
                );
            }
            GraphKind::DiskRead(_)
            | GraphKind::DiskWrite(_)
            | GraphKind::NetworkRx(_)
            | GraphKind::NetworkTx(_) => {
                //TODO: automatic Y scale for these graphs
                text(
                    "0 B/s",
                    Point::new(max_x, calc_y(0.0)),
                    Alignment::Left,
                    Vertical::Bottom,
                    &mut frame,
                );
                let format_options =
                    humansize::FormatSizeOptions::from(humansize::DECIMAL).decimal_places(0);
                for &value in &[scale_y * 0.2, scale_y * 0.4, scale_y * 0.6, scale_y * 0.8] {
                    let y = calc_y(value);
                    let path = canvas::Path::line(Point::new(min_x, y), Point::new(max_x, y));
                    frame.stroke(
                        &path,
                        canvas::Stroke::default().with_color(accent_color_0_5),
                    );

                    text(
                        &format!("{}/s", humansize::format_size(value as u64, format_options)),
                        Point::new(max_x, y),
                        Alignment::Left,
                        Vertical::Center,
                        &mut frame,
                    );
                }
                text(
                    &format!(
                        "{}/s",
                        humansize::format_size(scale_y as u64, format_options)
                    ),
                    Point::new(max_x, calc_y(scale_y)),
                    Alignment::Left,
                    Vertical::Top,
                    &mut frame,
                );
            }
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
                GraphKind::Cpu => graph_item.total_cpu_usage(),
                GraphKind::Memory => {
                    100.0 * (graph_item.memory.used as f32) / (graph_item.memory.total as f32)
                }
                GraphKind::Swap => {
                    100.0 * (graph_item.memory.swap_used as f32)
                        / (graph_item.memory.swap_total as f32)
                }
                GraphKind::GpuUsage(gpu_name) => {
                    let mut total = 0.0;
                    for gpu in graph_item.gpus.iter().filter(|x| x.name == gpu_name) {
                        total += gpu.usage.unwrap_or_default();
                    }
                    total
                }
                GraphKind::GpuVram(gpu_name) => {
                    let mut total = 0.0;
                    for gpu in graph_item.gpus.iter().filter(|x| x.name == gpu_name) {
                        total += 100.0 * (gpu.vram_used.unwrap_or_default() as f32)
                            / (gpu.vram_total.unwrap_or_default() as f32);
                    }
                    total
                }
                GraphKind::DiskRead(disk_name) => {
                    let mut total = 0.0;
                    for disk in graph_item.disks.iter().filter(|x| x.name == disk_name) {
                        total += disk.read as f32;
                    }
                    total
                }
                GraphKind::DiskWrite(disk_name) => {
                    let mut total = 0.0;
                    for disk in graph_item.disks.iter().filter(|x| x.name == disk_name) {
                        total += disk.write as f32;
                    }
                    total
                }
                GraphKind::NetworkRx(network_name) => {
                    let mut total = 0.0;
                    for network in graph_item
                        .networks
                        .iter()
                        .filter(|x| x.name == network_name)
                    {
                        total += network.rx as f32;
                    }
                    total
                }
                GraphKind::NetworkTx(network_name) => {
                    let mut total = 0.0;
                    for network in graph_item
                        .networks
                        .iter()
                        .filter(|x| x.name == network_name)
                    {
                        total += network.tx as f32;
                    }
                    total
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

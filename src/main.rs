// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use clap_lex::RawArgs;
use cosmic::{
    Application, ApplicationExt, Element,
    app::{Core, Settings, Task, context_drawer},
    cosmic_config::{self, CosmicConfigEntry},
    cosmic_theme, executor,
    iced::{
        self, Alignment, Border, Length, Limits, Size, Subscription,
        core::text::{Ellipsize, EllipsizeHeightLimit, Shaping},
        widget::{
            operation::AbsoluteOffset,
            scrollable::{Direction, Scrollbar, Viewport, scroll_to},
        },
    },
    surface, theme,
    widget::{
        self,
        about::About,
        canvas,
        menu::{action::MenuAction, key_bind::KeyBind},
        nav_bar, segmented_button,
        table::{ItemCategory, ItemInterface},
    },
};
use itertools::Itertools;
use regex::{Regex, RegexBuilder};
use std::{
    any::TypeId,
    collections::{HashMap, VecDeque},
    env,
    error::Error,
    time::{Duration, Instant},
};
use sysinfo::Pid;

use config::{AppTheme, CONFIG_VERSION, Config};
mod config;

use graph::{Graph, GraphKind};
mod graph;

use info::{GpuId, GraphItem, ProcessCategory, ProcessItem};
mod info;

mod localize;

use menu::menu_bar;
mod menu;

const SMALL_GRAPH_HEIGHT: f32 = 176.0;
const LARGE_GRAPH_HEIGHT: f32 = 300.0;
const MIN_GRAPH_WIDTH: f32 = 640.0;
const MIN_PROCESSES_WIDTH: f32 = 720.0;

#[rustfmt::skip]
fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let raw_args = RawArgs::from_args();
    let mut cursor = raw_args.cursor();
    while let Some(arg) = raw_args.next_os(&mut cursor) {
        match arg.to_str() {
            Some("--help") | Some("-h") => {
                print_help();
                return Ok(());
            }
            Some("--version") | Some("-V") => {
                println!(
                    "cosmic-monitor {}",
                    env!("CARGO_PKG_VERSION"),
                );
                return Ok(());
            }
            _ => {}
        }
    }

    localize::localize();

    let (config_handler, config) = match cosmic_config::Config::new(App::APP_ID, CONFIG_VERSION) {
        Ok(config_handler) => {
            let config = match Config::get_entry(&config_handler) {
                Ok(ok) => ok,
                Err((errs, config)) => {
                    log::info!("errors loading config: {:?}", errs);
                    config
                }
            };
            (Some(config_handler), config)
        }
        Err(err) => {
            log::error!("failed to create config handler: {}", err);
            (None, Config::default())
        }
    };

    let mut settings = Settings::default();
    settings = settings.theme(config.app_theme.theme());
    settings = settings.size_limits(Limits::NONE.min_width(360.0).min_height(180.0));

    let flags = Flags {
        config_handler,
        config,
    };

    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
}

fn format_frequency(mhz: u64) -> String {
    if mhz >= 1000 {
        format!("{:.2} GHz", (mhz as f64) / 1000.0)
    } else {
        format!("{} MHz", mhz)
    }
}

fn print_help() {
    println!(
        r#"COSMIC System Monitor
Designed for the COSMIC™ desktop environment, cosmic-monitor is a libcosmic-based system monitor.

Project home page: https://github.com/pop-os/cosmic-monitor
Options:
  --help                          Show this message
  --version                       Show the version of cosmic-monitor"#
    );
}

fn table_header(
    categories: &[ProcessCategory],
    sort_category: ProcessCategory,
    sort_direction: bool,
    sortable: bool,
) -> widget::Row<'static, Message, cosmic::Theme> {
    let mut header = widget::row::with_capacity(categories.len()).align_y(Alignment::Center);
    for &category in categories {
        let mut cat_row = widget::row::with_capacity(2).align_y(Alignment::Center);
        cat_row = cat_row.push(widget::text::heading(category.to_string()));
        if category == sort_category {
            cat_row = cat_row.push(
                widget::icon::from_name(if sort_direction {
                    "pan-up-symbolic"
                } else {
                    "pan-down-symbolic"
                })
                .size(16),
            );
        }
        let container = widget::container(cat_row)
            .align_x(category.data_align())
            .align_y(Alignment::Center)
            .padding([0, 8])
            .height(Length::Fixed(40.0))
            .width(category.width());
        if sortable {
            header =
                header.push(widget::mouse_area(container).on_press(Message::ProcessSort(category)));
        } else {
            header = header.push(container);
        }
    }
    header
}

fn table_row<'a>(
    item: &'a ProcessItem,
    categories: &[ProcessCategory],
    selected: &Option<Pid>,
) -> Element<'a, Message> {
    let cosmic_theme::Spacing { space_xxs, .. } = theme::active().cosmic().spacing;

    let mut row = widget::row::with_capacity(categories.len()).align_y(Alignment::Center);
    for &category in categories {
        let mut cat_row = widget::row::with_capacity(2)
            .align_y(Alignment::Center)
            .spacing(space_xxs);
        if let Some(icon) = item.get_icon(category) {
            cat_row = cat_row.push(icon);
        }
        let text = item.text(category);
        if !text.is_empty() {
            cat_row = cat_row.push(
                widget::text(text)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                    //TODO: should basic shaping only be used on some columns?
                    .shaping(Shaping::Basic),
            );
        }
        row = row.push(
            widget::container(cat_row)
                .align_x(category.data_align())
                .align_y(Alignment::Center)
                .padding([0, 8])
                .height(Length::Fixed(40.0))
                .width(category.width()),
        );
    }
    let mut container = widget::container(row);
    //TODO: allow App selection
    if selected.is_some() && selected == &item.pid {
        container = container.style(|theme| {
            let cosmic = theme.cosmic();
            widget::container::Style {
                text_color: Some(cosmic.on_accent_color().into()),
                background: Some(cosmic.accent_color().into()),
                border: Border {
                    radius: cosmic.corner_radii.radius_xs.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });
    }
    widget::mouse_area(container)
        .on_press(Message::ProcessSelect(item.pid))
        .into()
}

#[derive(Clone, Debug)]
pub struct Flags {
    config_handler: Option<cosmic_config::Config>,
    config: Config,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    About,
    Settings,
}

impl Action {
    fn message(&self, _entity_opt: Option<segmented_button::Entity>) -> Message {
        match self {
            Self::None => Message::None,
            Self::About => Message::ToggleContextPage(ContextPage::About),
            Self::Settings => Message::ToggleContextPage(ContextPage::Settings),
        }
    }
}

impl MenuAction for Action {
    type Message = Message;

    fn message(&self) -> Message {
        self.message(None)
    }
}

#[derive(Clone, Debug)]
pub enum DialogKind {
    ProcessQuit { name: String, pid: Pid, force: bool },
}

/// Messages that are used specifically by our [`App`].
#[derive(Clone, Debug)]
pub enum Message {
    None,
    AppTheme(AppTheme),
    Config(Box<Config>),
    DialogCancel,
    DialogConfirm,
    DialogOpen(DialogKind),
    GpuSelect(usize),
    Graph(GraphItem),
    LaunchUrl(String),
    NavPage(NavPage),
    ProcessSearch(String),
    ProcessSelect(Option<Pid>),
    ProcessSort(ProcessCategory),
    ScrollHeader(Viewport),
    ScrollTable(Viewport),
    SeeAllProcesses(bool, ProcessCategory, bool),
    Snapshot(GraphItem, Vec<ProcessItem>, Vec<ProcessItem>),
    Surface(surface::Action),
    SystemThemeChange,
    ToggleContextPage(ContextPage),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextPage {
    About,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavPage {
    Dashboard,
    Applications,
    Processes,
    Cpu,
    Memory,
    Gpu,
    Disk,
    Network,
}

impl NavPage {
    pub fn all() -> &'static [Self] {
        &[
            Self::Dashboard,
            Self::Applications,
            Self::Processes,
            Self::Cpu,
            Self::Memory,
            Self::Gpu,
            Self::Disk,
            Self::Network,
        ]
    }

    pub fn title(&self) -> String {
        match self {
            Self::Dashboard => fl!("dashboard"),
            Self::Applications => fl!("applications"),
            Self::Processes => fl!("processes"),
            Self::Cpu => fl!("cpu"),
            Self::Memory => fl!("memory"),
            Self::Gpu => fl!("gpu"),
            Self::Disk => fl!("disk"),
            Self::Network => fl!("network"),
        }
    }
}

/// The [`App`] stores application-specific state.
pub struct App {
    about: About,
    app_themes: Vec<String>,
    apps: Vec<ProcessItem>,
    config: Config,
    config_handler: Option<cosmic_config::Config>,
    context_page: ContextPage,
    core: Core,
    dialog_opt: Option<DialogKind>,
    gpu_id_opt: Option<GpuId>,
    gpu_names: Vec<String>,
    graph_history: VecDeque<GraphItem>,
    graph_snapshot: Option<GraphItem>,
    key_binds: HashMap<KeyBind, Action>,
    nav_model: segmented_button::SingleSelectModel,
    processes: Vec<ProcessItem>,
    process_content: iced::widget::list::Content<ProcessItem>,
    process_search: (String, Option<Regex>),
    process_selected: Option<Pid>,
    process_sort: (ProcessCategory, bool),
    scroll_header_id: widget::Id,
    scroll_table_id: widget::Id,
}

impl App {
    fn update_config(&mut self) -> Task<Message> {
        let theme = self.config.app_theme.theme();
        cosmic::command::set_theme(theme)
    }

    fn update_snapshot(&mut self) {
        self.gpu_names.clear();
        if let Some(graph_item) = &self.graph_snapshot {
            for gpu in graph_item.gpus.iter() {
                self.gpu_names.push(gpu.name.clone());
            }
        }

        let list = if matches!(
            self.nav_model.active_data::<NavPage>(),
            Some(NavPage::Applications)
        ) {
            &mut self.apps
        } else {
            &mut self.processes
        };
        list.sort_by(|a, b| {
            if self.process_sort.1 {
                b.compare(a, self.process_sort.0)
            } else {
                a.compare(b, self.process_sort.0)
            }
        });

        let mut i = 0;
        for item in list.iter() {
            if let Some(regex) = &self.process_search.1 {
                if !item.matches(&regex) {
                    continue;
                }
            }
            if i >= self.process_content.len() {
                self.process_content.push(item.clone());
            } else if self.process_content.get(i) != Some(&item) {
                *self.process_content.get_mut(i).unwrap() = item.clone();
            }
            i += 1;
        }
        while i < self.process_content.len() {
            self.process_content.remove(i);
        }
    }

    fn settings(&self) -> Element<'_, Message> {
        let app_theme_selected = match self.config.app_theme {
            AppTheme::Dark => 1,
            AppTheme::Light => 2,
            AppTheme::System => 0,
        };
        let appearance_section = widget::settings::section().title(fl!("appearance")).add(
            widget::settings::item::builder(fl!("theme")).control(widget::dropdown(
                &self.app_themes,
                Some(app_theme_selected),
                move |index| {
                    Message::AppTheme(match index {
                        1 => AppTheme::Dark,
                        2 => AppTheme::Light,
                        _ => AppTheme::System,
                    })
                },
            )),
        );

        widget::settings::view_column(vec![appearance_section.into()]).into()
    }

    fn responsive_graph_top_processes<'a>(
        &'a self,
        category: ProcessCategory,
        f: impl Fn() -> Element<'a, Message> + 'a,
    ) -> Element<'a, Message> {
        let cosmic_theme::Spacing {
            space_xxl, space_l, ..
        } = theme::active().cosmic().spacing;

        widget::responsive(move |size| {
            let graph = f();
            if size.width > MIN_GRAPH_WIDTH + space_xxl as f32 + MIN_PROCESSES_WIDTH {
                widget::row!(
                    graph,
                    widget::container(self.top_processes_by(false, category, false, false, 7))
                        .width(MIN_PROCESSES_WIDTH)
                )
                .spacing(space_xxl)
                .into()
            } else {
                widget::column!(
                    graph,
                    self.top_processes_by(false, category, false, false, 5)
                )
                .spacing(space_l)
                .into()
            }
        })
        .into()
    }

    fn top_processes_by<'a>(
        &'a self,
        show_apps: bool,
        sort_category: ProcessCategory,
        sort_direction: bool,
        sortable: bool,
        count: usize,
    ) -> Element<'a, Message> {
        let cosmic_theme::Spacing { space_xxs, .. } = theme::active().cosmic().spacing;

        let categories = ProcessCategory::for_top_processes(sort_category);
        let mut column = widget::column::with_capacity(count + 2);
        column = column.push(table_header(
            &categories,
            sort_category,
            sort_direction,
            sortable,
        ));
        for item in if show_apps {
            &self.apps
        } else {
            &self.processes
        }
        .iter()
        .k_smallest_by(count, |a, b| {
            if sort_direction {
                b.compare(a, sort_category)
            } else {
                a.compare(b, sort_category)
            }
        }) {
            column = column.push(
                widget::column::with_capacity(2)
                    .push(widget::divider::horizontal::default())
                    .push(table_row(item, &categories, &self.process_selected)),
            );
        }
        column = column.push(widget::divider::horizontal::default());
        widget::column!(
            widget::text::title4(if show_apps {
                fl!("applications")
            } else {
                fl!("processes")
            }),
            column,
            widget::button::text(if show_apps {
                fl!("see-all-applications")
            } else {
                fl!("see-all-processes")
            })
            .trailing_icon(widget::icon::from_name("go-next-symbolic"))
            .on_press(Message::SeeAllProcesses(
                show_apps,
                sort_category,
                sort_direction
            )),
        )
        .spacing(space_xxs)
        .into()
    }

    fn view_dashboard<'a>(&'a self, graph_item: &'a GraphItem, size: Size) -> Element<'a, Message> {
        let cosmic_theme::Spacing {
            space_xl,
            space_s,
            space_xs,
            space_xxs,
            ..
        } = theme::active().cosmic().spacing;

        let card = |graph_kind,
                    name,
                    data,
                    caption,
                    process_category: Option<ProcessCategory>,
                    message: Message|
         -> Element<Message> {
            let mut column = widget::column::with_capacity(7)
                .spacing(space_xxs)
                .push(widget::text::title4(name))
                .push(widget::column!(
                    widget::text::body(data)
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                    widget::text::caption(caption)
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                ));

            if let Some(sort_category) = process_category {
                // The compare function is backwards, so this uses min_by
                if let Some(item) = self
                    .processes
                    .iter()
                    .min_by(|a, b| a.compare(b, sort_category))
                {
                    let mut row = widget::row::with_capacity(3)
                        .align_y(Alignment::Center)
                        .spacing(space_xxs);
                    if let Some(icon) = item.get_icon(ProcessCategory::App) {
                        row = row.push(icon);
                    }
                    row = row
                        .push(
                            widget::container(
                                widget::text(&item.name)
                                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                                    .shaping(Shaping::Basic),
                            )
                            .align_x(Alignment::Start)
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        )
                        .push(
                            widget::container(
                                widget::text(item.text(sort_category)).shaping(Shaping::Basic),
                            )
                            .align_x(Alignment::End)
                            .align_y(Alignment::Center)
                            .width(Length::Shrink),
                        );
                    column = column
                        .push(widget::divider::horizontal::default())
                        .push(row)
                        .push(widget::divider::horizontal::default());
                }
            } else if matches!(graph_kind, GraphKind::NetworkTotal) {
                if let Some((name, io)) = graph_item
                    .networks
                    .iter()
                    .map(|x| (x.name.as_str(), (x.rx + x.tx) as u64))
                    .max_by(|a, b| a.1.cmp(&b.1))
                {
                    let mut row = widget::row::with_capacity(2).align_y(Alignment::Center);
                    row = row
                        .push(
                            widget::container(
                                widget::text(name)
                                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                                    .shaping(Shaping::Basic),
                            )
                            .align_x(Alignment::Start)
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        )
                        .push(
                            widget::container(
                                widget::text(format!(
                                    "{}/s",
                                    humansize::format_size(io, humansize::DECIMAL)
                                ))
                                .shaping(Shaping::Basic),
                            )
                            .align_x(Alignment::End)
                            .align_y(Alignment::Center)
                            .width(Length::Shrink),
                        );
                    column = column
                        .push(widget::divider::horizontal::default())
                        .push(row)
                        .push(widget::divider::horizontal::default());
                }
            }

            column = column.push(
                widget::button::text(fl!("details"))
                    .trailing_icon(widget::icon::from_name("go-next-symbolic"))
                    .on_press(message),
            );

            widget::container(
                widget::row!(
                    canvas(Graph::new(graph_kind, &self.graph_history).border())
                        .height(SMALL_GRAPH_HEIGHT)
                        .width(Length::Fill),
                    column.width(Length::Fill)
                )
                .spacing(space_xs),
            )
            .class(theme::Container::Card)
            .padding(space_s)
            .width(Length::Fill)
            .into()
        };

        let mut items = Vec::with_capacity(4 + graph_item.gpus.len() * 2);
        items.push(card(
            GraphKind::Cpu,
            fl!("cpu"),
            if let Some(temp) = graph_item.max_cpu_temp() {
                format!(
                    "{:.1}% / {} / {:.1}°C",
                    graph_item.total_cpu_usage(),
                    format_frequency(graph_item.max_cpu_frequency()),
                    temp
                )
            } else {
                format!(
                    "{:.1}% / {}",
                    graph_item.total_cpu_usage(),
                    format_frequency(graph_item.max_cpu_frequency())
                )
            },
            graph_item
                .cpus
                .first()
                .map(|x| x.brand.clone())
                .unwrap_or_default(),
            Some(ProcessCategory::CPU),
            Message::NavPage(NavPage::Cpu),
        ));

        items.push(card(
            GraphKind::Memory,
            fl!("memory"),
            format!(
                "{:.1}% / {}",
                100.0 * (graph_item.memory.used as f32) / (graph_item.memory.total as f32),
                humansize::format_size(graph_item.memory.used, humansize::BINARY),
            ),
            format!(
                "{}",
                humansize::format_size(graph_item.memory.total, humansize::BINARY),
            ),
            Some(ProcessCategory::Memory),
            Message::NavPage(NavPage::Memory),
        ));

        let disk_io = graph_item.total_disk_io();
        items.push(card(
            GraphKind::DiskTotal,
            fl!("disk"),
            format!(
                "{}/s read / {}/s write",
                humansize::format_size(disk_io.0 as u64, humansize::DECIMAL),
                humansize::format_size(disk_io.1 as u64, humansize::DECIMAL)
            ),
            String::new(),
            Some(ProcessCategory::DiskTotal),
            Message::NavPage(NavPage::Disk),
        ));

        let network_io = graph_item.total_network_io();
        items.push(card(
            GraphKind::NetworkTotal,
            fl!("network"),
            format!(
                "{}/s rx / {}/s tx",
                humansize::format_size(network_io.0 as u64, humansize::DECIMAL),
                humansize::format_size(network_io.1 as u64, humansize::DECIMAL)
            ),
            String::new(),
            None,
            Message::NavPage(NavPage::Network),
        ));

        for (gpu_i, gpu) in graph_item.gpus.iter().enumerate() {
            if let Some(usage) = gpu.usage {
                items.push(card(
                    GraphKind::GpuUsage(gpu.id),
                    fl!("gpu-index", index = gpu_i),
                    if let Some(temp) = gpu.temp {
                        format!("{:.1}% / {:.1}°C", usage, temp)
                    } else {
                        format!("{:.1}%", usage)
                    },
                    gpu.name.clone(),
                    Some(ProcessCategory::GpuUsage(gpu.id, Some(gpu_i))),
                    Message::GpuSelect(gpu_i),
                ));
            }
            if let Some(vram_used) = gpu.vram_used {
                if let Some(vram_total) = gpu.vram_total {
                    items.push(card(
                        GraphKind::GpuVram(gpu.id),
                        fl!("gpu-vram-index", index = gpu_i),
                        format!(
                            "{:.1}% / {}",
                            100.0 * (vram_used as f32) / (vram_total as f32),
                            humansize::format_size(vram_used, humansize::BINARY),
                        ),
                        //TODO: show vram total format!("{}", humansize::format_size(vram_total, humansize::BINARY)),
                        gpu.name.clone(),
                        Some(ProcessCategory::GpuVram(gpu.id, Some(gpu_i))),
                        Message::GpuSelect(gpu_i),
                    ));
                }
            }
        }

        let card_height = space_s as f32 + SMALL_GRAPH_HEIGHT + space_s as f32;
        let min_width = 440.0;
        let content_width = size.width - (space_xl * 2) as f32;
        enum DashboardLayout {
            Small,
            Medium,
            Large,
        }
        let large_width = content_width - (MIN_PROCESSES_WIDTH + space_s as f32) * 2.0;
        let large_cards = (large_width / min_width).floor()
            * (size.height / (card_height + space_s as f32)).floor();
        // Make sure there is enough space for all cards before attempting large layout
        let (graphs_width, layout) = if large_cards >= items.len() as f32 {
            (large_width, DashboardLayout::Large)
        } else if content_width > MIN_PROCESSES_WIDTH + space_s as f32 + min_width {
            (
                content_width - (MIN_PROCESSES_WIDTH + space_s as f32),
                DashboardLayout::Medium,
            )
        } else {
            (content_width, DashboardLayout::Small)
        };

        let mut cols = 1;
        while cols < 4 && graphs_width / ((cols + 1) as f32) > min_width {
            cols += 1;
        }
        let rows = (items.len() + cols - 1) / cols;
        let mut column = widget::column::with_capacity(rows).spacing(space_s);

        // Graphs
        let mut row = widget::row::with_capacity(cols).spacing(space_s);
        let mut col = 0;
        for item in items {
            if col >= cols {
                column = column.push(row);
                row = widget::row::with_capacity(cols).spacing(space_s);
                col = 0;
            }
            row = row.push(item);
            col += 1;
        }
        if col > 0 {
            while col < cols {
                row = row.push(widget::space().width(Length::Fill));
                col += 1;
            }
            column = column.push(row);
        }

        // Top apps/processes
        let mut list_cards = Vec::with_capacity(2);
        let mut app_count = 5;
        let mut proc_count = 5;
        if matches!(layout, DashboardLayout::Medium | DashboardLayout::Large) {
            let rows_height =
                rows as f32 * card_height + (space_s as f32) * rows.saturating_sub(1) as f32;
            while app_count < 50 && proc_count < 50 {
                let list_height = |list_count: u16| -> f32 {
                    (space_s
                    + 30 /* title 4 */
                    + space_s
                    + 24 /* header */ + 1 /* divider */
                    + ((40 + 1) * list_count) /* items */
                    + space_s
                    + 32 /* button */
                    + space_s) as f32
                };
                let (mut next_app_count, next_proc_count) = if app_count < proc_count {
                    (app_count + 1, proc_count)
                } else {
                    (app_count, proc_count + 1)
                };
                let total_height = match layout {
                    DashboardLayout::Large => {
                        // Sync the list sizes since they will be side by side
                        next_app_count = next_proc_count;
                        list_height(next_app_count)
                    }
                    DashboardLayout::Medium => {
                        list_height(next_app_count)
                            + (space_s as f32)
                            + list_height(next_proc_count)
                    }
                    DashboardLayout::Small => break,
                };
                if total_height > rows_height.max(size.height) {
                    break;
                }
                app_count = next_app_count;
                proc_count = next_proc_count;
            }
        }
        for &(show_apps, list_count) in &[(true, app_count), (false, proc_count)] {
            list_cards.push(Element::from(
                widget::container(
                    widget::column!(self.top_processes_by(
                        show_apps,
                        self.process_sort.0,
                        self.process_sort.1,
                        true,
                        list_count as usize,
                    ))
                    .spacing(space_s),
                )
                .class(theme::Container::Card)
                .padding(space_s)
                .width(Length::Fill),
            ));
        }

        let content: Element<Message> = match layout {
            DashboardLayout::Large => {
                // Top apps/processes as row next to graphs
                widget::row!(
                    widget::row::with_children(list_cards)
                        .spacing(space_s)
                        .width(Length::Fixed(MIN_PROCESSES_WIDTH * 2.0 + space_s as f32)),
                    column
                )
                .spacing(space_s)
                .into()
            }
            DashboardLayout::Medium => {
                // Top apps/processes as column next to graphs
                widget::row!(
                    widget::column::with_children(list_cards)
                        .spacing(space_s)
                        .width(Length::Fixed(MIN_PROCESSES_WIDTH)),
                    column
                )
                .spacing(space_s)
                .into()
            }
            DashboardLayout::Small => {
                // Top apps/processes as column above graphs
                widget::column!(
                    widget::column::with_children(list_cards).spacing(space_s),
                    column
                )
                .spacing(space_s)
                .into()
            }
        };

        widget::mouse_area(
            widget::scrollable(
                widget::container(content)
                    .padding([0, space_xl, space_s, space_xl])
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .on_press(Message::ProcessSelect(None))
        .into()
    }
}

/// Implement [`Application`] to integrate with COSMIC.
impl Application for App {
    /// Default async executor to use with the app.
    type Executor = executor::Default;

    /// Argument received
    type Flags = Flags;

    /// Message type specific to our [`App`].
    type Message = Message;

    /// The unique application ID to supply to the window manager.
    const APP_ID: &'static str = "com.system76.CosmicMonitor";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    /// Creates the application, and optionally emits command on initialize.
    fn init(mut core: Core, flags: Self::Flags) -> (Self, Task<Self::Message>) {
        core.window.context_is_overlay = false;
        core.nav_bar_set_toggled(false);

        let app_themes = vec![fl!("match-desktop"), fl!("dark"), fl!("light")];

        let about = About::default()
            .name(fl!("app-name"))
            .icon(widget::icon::from_name(Self::APP_ID))
            .version(env!("CARGO_PKG_VERSION"))
            .author("System76")
            .comments(fl!("comment"))
            .license("GPL-3.0-only")
            .license_url("https://spdx.org/licenses/GPL-3.0-only")
            .links([
                (
                    fl!("repository"),
                    "https://github.com/pop-os/cosmic-monitor",
                ),
                (
                    fl!("support"),
                    "https://github.com/pop-os/cosmic-monitor/issues",
                ),
            ]);

        let mut nav_model = nav_bar::Model::builder();
        for &page in NavPage::all() {
            nav_model = nav_model.insert(|mut b| {
                if matches!(page, NavPage::Dashboard) {
                    b = b.activate();
                }
                b.text(page.title())
                    .data::<NavPage>(page)
                    .data::<widget::Id>(widget::Id::unique())
            });
        }

        let mut app = Self {
            about,
            app_themes,
            apps: Vec::new(),
            config: flags.config,
            config_handler: flags.config_handler,
            context_page: ContextPage::Settings,
            core,
            dialog_opt: None,
            gpu_id_opt: None,
            gpu_names: Vec::new(),
            graph_history: VecDeque::new(),
            graph_snapshot: None,
            key_binds: HashMap::new(),
            nav_model: nav_model.build(),
            processes: Vec::new(),
            process_content: iced::widget::list::Content::new(),
            process_search: (String::new(), None),
            process_selected: None,
            process_sort: (ProcessCategory::default(), false),
            scroll_header_id: widget::Id::unique(),
            scroll_table_id: widget::Id::unique(),
        };

        let command = Task::batch([app.update_config(), app.set_window_title(fl!("app-name"))]);
        (app, command)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav_model)
    }

    //TODO: currently the first escape unfocuses, and the second calls this function
    fn on_escape(&mut self) -> Task<Message> {
        if self.dialog_opt.take().is_some() {
            return Task::none();
        }
        if self.core.window.show_context {
            return self.update(Message::ToggleContextPage(self.context_page));
        }
        if self.process_selected.take().is_some() {
            return Task::none();
        }
        if !self.process_search.0.is_empty() || self.process_search.1.is_some() {
            return self.update(Message::ProcessSearch(String::new()));
        }
        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Self::Message> {
        self.nav_model.activate(id);
        self.process_selected = None;
        self.update_snapshot();
        Task::none()
    }

    /// Handle application events here.
    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        // Helper for updating config values efficiently
        macro_rules! config_set {
            ($name: ident, $value: expr) => {
                match &self.config_handler {
                    Some(config_handler) => {
                        if let Err(err) =
                            paste::paste! { self.config.[<set_ $name>](config_handler, $value) }
                        {
                            log::warn!("failed to save config {:?}: {}", stringify!($name), err);
                        }
                    }
                    None => {
                        self.config.$name = $value;
                        log::warn!(
                            "failed to save config {:?}: no config handler",
                            stringify!($name)
                        );
                    }
                }
            };
        }
        match message {
            Message::None => {}
            Message::AppTheme(app_theme) => {
                config_set!(app_theme, app_theme);
                return self.update_config();
            }
            Message::Config(config) => {
                if *config != self.config {
                    self.config = *config;
                    return self.update_config();
                }
            }
            Message::DialogCancel => {
                self.dialog_opt = None;
            }
            Message::DialogConfirm => {
                if let Some(dialog_kind) = self.dialog_opt.take() {
                    match dialog_kind {
                        DialogKind::ProcessQuit { pid, force, .. } => {
                            //TODO: show errors?
                            #[cfg(unix)]
                            {
                                if let Ok(pid_c) = pid.as_u32().try_into() {
                                    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
                                    unsafe {
                                        libc::kill(pid_c, sig);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Message::DialogOpen(dialog_kind) => {
                self.dialog_opt = Some(dialog_kind);
            }
            Message::GpuSelect(gpu_i) => {
                self.gpu_id_opt = None;
                if let Some(graph_item) = &self.graph_snapshot {
                    if let Some(gpu) = graph_item.gpus.get(gpu_i) {
                        self.gpu_id_opt = Some(gpu.id);
                    }
                }
                return self.update(Message::NavPage(NavPage::Gpu));
            }
            Message::Graph(graph_item) => {
                self.graph_history.push_back(graph_item);
                let now = Instant::now();
                self.graph_history
                    .retain(|x| now.saturating_duration_since(x.time) < Duration::from_secs(60));
            }
            Message::LaunchUrl(url) => {
                if let Err(err) = open::that_detached(&url) {
                    log::warn!("failed to open {:?}: {}", url, err);
                }
            }
            Message::NavPage(nav_page) => {
                let mut id_opt = None;
                for id in self.nav_model.iter() {
                    if self.nav_model.data::<NavPage>(id) == Some(&nav_page) {
                        id_opt = Some(id);
                        break;
                    }
                }
                if let Some(id) = id_opt {
                    self.nav_model.activate(id);
                    self.process_selected = None;
                    self.update_snapshot();
                }
            }
            Message::ProcessSearch(search) => {
                let regex_opt = if !search.is_empty() {
                    RegexBuilder::new(&regex::escape(&search))
                        .case_insensitive(true)
                        .build()
                        .ok()
                } else {
                    None
                };
                self.process_search = (search, regex_opt);
                self.update_snapshot();
            }
            Message::ProcessSelect(process_selected) => {
                self.process_selected = process_selected;
                //TODO: reset that item in Contents?
            }
            Message::ProcessSort(category) => {
                if self.process_sort.0 == category {
                    self.process_sort.1 = !self.process_sort.1
                } else {
                    self.process_sort.0 = category;
                    self.process_sort.1 = false;
                }
                self.update_snapshot();
            }
            Message::ScrollHeader(scroll) => {
                // Sync header horizontal scroll to table
                return scroll_to(
                    self.scroll_table_id.clone(),
                    AbsoluteOffset {
                        x: Some(scroll.absolute_offset().x),
                        y: None,
                    },
                );
            }
            Message::ScrollTable(scroll) => {
                // Sync table horizontal scroll to header
                return scroll_to(
                    self.scroll_header_id.clone(),
                    AbsoluteOffset {
                        x: Some(scroll.absolute_offset().x),
                        y: None,
                    },
                );
            }
            Message::SeeAllProcesses(show_apps, category, direction) => {
                self.process_sort = (category, direction);
                self.update_snapshot();
                return self.update(Message::NavPage(if show_apps {
                    NavPage::Applications
                } else {
                    NavPage::Processes
                }));
            }
            Message::Snapshot(graph_item, apps, processes) => {
                self.graph_snapshot = Some(graph_item);
                self.apps = apps;
                self.processes = processes;
                self.update_snapshot();
            }
            Message::Surface(a) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(a),
                ));
            }
            Message::SystemThemeChange => {
                return self.update_config();
            }
            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
            }
        }

        Task::none()
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |s| Message::LaunchUrl(s.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
            ContextPage::Settings => context_drawer::context_drawer(
                self.settings(),
                Message::ToggleContextPage(ContextPage::Settings),
            )
            .title(fl!("settings")),
        })
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        let mut dialog = widget::dialog().secondary_action(
            widget::button::standard(fl!("cancel")).on_press(Message::DialogCancel),
        );
        match self.dialog_opt.as_ref()? {
            DialogKind::ProcessQuit { name, force, .. } => {
                dialog = dialog
                    .title(if *force {
                        fl!("force-quit-title")
                    } else {
                        fl!("quit-title")
                    })
                    .body(if *force {
                        fl!("force-quit-body", name = name)
                    } else {
                        fl!("quit-body", name = name)
                    })
                    .primary_action(
                        widget::button::destructive(if *force {
                            fl!("force-quit")
                        } else {
                            fl!("quit")
                        })
                        .on_press(Message::DialogConfirm),
                    );
            }
        }
        Some(dialog.into())
    }

    fn footer(&self) -> Option<Element<'_, Self::Message>> {
        let cosmic_theme::Spacing { space_xxs, .. } = theme::active().cosmic().spacing;

        let pid = self.process_selected?;
        let item = self.processes.iter().find(|x| x.pid == Some(pid))?;
        let mut row = widget::row::with_capacity(5)
            .align_y(Alignment::Center)
            .spacing(space_xxs);
        if let Some(icon) = item.get_icon(ProcessCategory::App) {
            row = row.push(icon);
        }
        row = row
            .push(
                widget::container(
                    widget::text(&item.name)
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                        .shaping(Shaping::Basic),
                )
                .align_x(Alignment::Start)
                .align_y(Alignment::Center)
                .width(Length::Fill),
            )
            .push(
                widget::container(
                    widget::text(item.text(ProcessCategory::PID)).shaping(Shaping::Basic),
                )
                .align_x(Alignment::End)
                .align_y(Alignment::Center)
                .width(Length::Shrink),
            )
            .push(
                widget::button::destructive(fl!("force-quit")).on_press(Message::DialogOpen(
                    DialogKind::ProcessQuit {
                        name: item.name.clone(),
                        pid,
                        force: true,
                    },
                )),
            )
            .push(
                widget::button::standard(fl!("quit")).on_press(Message::DialogOpen(
                    DialogKind::ProcessQuit {
                        name: item.name.clone(),
                        pid,
                        force: false,
                    },
                )),
            );
        Some(
            widget::container(row)
                .padding(space_xxs)
                .class(theme::Container::Card)
                .into(),
        )
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![menu_bar(&self.core, &self.config, &self.key_binds)]
    }

    /// Creates a view after each update.
    fn view(&self) -> Element<'_, Self::Message> {
        let cosmic_theme::Spacing {
            space_xxl,
            space_xl,
            space_l,
            space_m,
            space_s,
            space_xs,
            space_xxs,
            ..
        } = theme::active().cosmic().spacing;

        let nav_page = self
            .nav_model
            .active_data::<NavPage>()
            .map_or(NavPage::Dashboard, |x| *x);
        let mut page_header = widget::column::with_capacity(6).padding([0, space_xl]);
        page_header = page_header
            .push(
                widget::button::text(fl!("dashboard"))
                    .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                    .on_press(Message::NavPage(NavPage::Dashboard)),
            )
            .push(widget::column!(widget::text::title2(nav_page.title())))
            .push(widget::space().height(space_m));
        let content: Element<Message> = match (nav_page, &self.graph_snapshot) {
            (NavPage::Dashboard, Some(graph_item)) => {
                let content = widget::responsive(|size| self.view_dashboard(graph_item, size))
                    .height(Length::Fill)
                    .width(Length::Fill);
                // view_dashboard will do its own container so it can know correct window height
                return if let Some(id) = self.nav_model.active_data::<widget::Id>() {
                    widget::id_container(content, id.clone()).into()
                } else {
                    content.into()
                };
            }
            (NavPage::Applications | NavPage::Processes, _) => {
                page_header = page_header
                    .push(
                        widget::container(
                            widget::search_input(fl!("search-processes"), &self.process_search.0)
                                .on_clear(Message::ProcessSearch(String::new()))
                                .on_input(Message::ProcessSearch)
                                .width(360.0),
                        )
                        .align_x(Alignment::Center)
                        .width(Length::Fill),
                    )
                    .push(widget::space().height(space_m));

                let responsive = widget::responsive(move |size| {
                    //TODO: table is too slow, this uses list to emulate table
                    let categories = match nav_page {
                        NavPage::Applications => {
                            ProcessCategory::for_applications(self.process_sort.0)
                        }
                        _ => ProcessCategory::for_processes(self.process_sort.0),
                    };
                    let (width, direction) = if size.width < 1000.0 {
                        (
                            1000.0,
                            Direction::Both {
                                vertical: Scrollbar::new(),
                                horizontal: Scrollbar::new(),
                            },
                        )
                    } else {
                        (size.width, Direction::Vertical(Scrollbar::new()))
                    };
                    let header =
                        table_header(&categories, self.process_sort.0, self.process_sort.1, true);
                    widget::column!(
                        iced::widget::scrollable(
                            header.padding([0, space_xl, 0, space_xl]).width(width),
                        )
                        .direction(Direction::Horizontal(Scrollbar::hidden()))
                        .id(self.scroll_header_id.clone())
                        .on_scroll(Message::ScrollHeader)
                        .width(Length::Fill),
                        widget::scrollable(
                            widget::container(iced::widget::List::new(
                                &self.process_content,
                                move |_i, item| {
                                    widget::column::with_capacity(2)
                                        .push(widget::divider::horizontal::default())
                                        .push(table_row(item, &categories, &self.process_selected))
                                        .into()
                                },
                            ))
                            .padding([0, space_xl, 0, space_xl])
                            .width(width),
                        )
                        .direction(direction)
                        .id(self.scroll_table_id.clone())
                        .on_scroll(Message::ScrollTable)
                    )
                    .padding([0, 0, space_xxs, 0])
                    .into()
                });

                // Custom view for horizontal scrolling
                let content = widget::mouse_area(
                    widget::column!(page_header, responsive,)
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .on_press(Message::ProcessSelect(None));
                return if let Some(id) = self.nav_model.active_data::<widget::Id>() {
                    widget::id_container(content, id.clone()).into()
                } else {
                    content.into()
                };
            }
            (NavPage::Cpu, Some(graph_item)) => {
                let mut column = widget::column::with_capacity(2)
                    .spacing(space_l)
                    .width(Length::Fill);

                // Overall utilization and top processes
                column = column.push(self.responsive_graph_top_processes(
                    ProcessCategory::CPU,
                    move || {
                        widget::column!(
                            widget::text::title4(fl!("overall-utilization")),
                            widget::row!(
                                widget::column!(
                                    widget::text::body(fl!("utilization")),
                                    widget::text::heading(format!(
                                        "{:.1}%",
                                        graph_item.total_cpu_usage()
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("speed")),
                                    widget::text::heading(format_frequency(
                                        graph_item.max_cpu_frequency()
                                    ))
                                ),
                                if let Some(temp) = graph_item.max_cpu_temp() {
                                    widget::column!(
                                        widget::text::body(fl!("temperature")),
                                        widget::text::heading(format!("{:.1}°C", temp))
                                    )
                                } else {
                                    widget::column!()
                                }
                            )
                            .spacing(space_m),
                            canvas(Graph::new(GraphKind::Cpu, &self.graph_history).legend())
                                .height(LARGE_GRAPH_HEIGHT)
                                .width(Length::Fill),
                        )
                        .spacing(space_xxs)
                        .into()
                    },
                ));

                // Utilization per core
                let mut children = Vec::with_capacity(graph_item.cpus.len());
                for cpu in graph_item.cpus.iter() {
                    children.push(
                        widget::column!(
                            widget::row!(
                                widget::text::heading(&cpu.name),
                                widget::space().width(Length::Fill),
                                widget::text::body(format_frequency(cpu.frequency))
                                    .align_x(Alignment::End),
                            )
                            .width(200.0 + 48.0),
                            widget::row!(
                                widget::determinate_linear(cpu.usage / 100.0)
                                    .girth(12.0)
                                    .width(200.0),
                                widget::text(format!("{:.1}%", cpu.usage))
                                    .align_x(Alignment::End)
                                    .width(48.0),
                            )
                            .align_y(Alignment::Center)
                        )
                        .into(),
                    );
                }
                column = column.push(
                    widget::column!(
                        widget::text::title4(fl!("utilization-per-core")),
                        widget::flex_row(children)
                            .column_spacing(space_m)
                            .row_spacing(space_xs)
                    )
                    .spacing(space_xxs),
                );

                column.into()
            }
            (NavPage::Memory, Some(graph_item)) => {
                let mem = &graph_item.memory;

                let mut column = widget::column::with_capacity(2)
                    .spacing(space_l)
                    .width(Length::Fill);

                // Memory information and top processes
                column = column.push(self.responsive_graph_top_processes(
                    ProcessCategory::Memory,
                    move || {
                        widget::column!(
                            widget::text::title4(fl!("memory-usage")),
                            widget::row!(
                                widget::column!(
                                    widget::text::body(fl!("capacity")),
                                    widget::text::heading(
                                        humansize::format_size(mem.total, humansize::BINARY)
                                            .to_string()
                                    )
                                ),
                                widget::column!(
                                    widget::text::body(fl!("in-use")),
                                    widget::text::heading(format!(
                                        "{} ({:.1}%)",
                                        humansize::format_size(mem.used, humansize::BINARY),
                                        100.0 * (mem.used as f64) / (mem.total as f64)
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("cache")),
                                    widget::text::heading(format!(
                                        "{} ({:.1}%)",
                                        humansize::format_size(mem.cache, humansize::BINARY),
                                        100.0 * (mem.cache as f64) / (mem.total as f64)
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("total-utilization")),
                                    widget::text::heading({
                                        let total_used = mem.used + mem.cache;
                                        format!(
                                            "{} ({:.1}%)",
                                            humansize::format_size(total_used, humansize::BINARY),
                                            100.0 * (total_used as f64) / (mem.total as f64)
                                        )
                                    })
                                ),
                            )
                            .spacing(space_m),
                            canvas(Graph::new(GraphKind::Memory, &self.graph_history).legend())
                                .height(LARGE_GRAPH_HEIGHT)
                                .width(Length::Fill),
                        )
                        .spacing(space_xxs)
                        .into()
                    },
                ));

                // Swap information (responsive, but no top processes)
                column = column.push(widget::responsive(move |size| {
                    let graph = widget::column!(
                        widget::text::title4(fl!("swap-usage")),
                        widget::row!(
                            widget::column!(
                                widget::text::body(fl!("capacity")),
                                widget::text::heading(
                                    humansize::format_size(mem.swap_total, humansize::BINARY)
                                        .to_string()
                                )
                            ),
                            widget::column!(
                                widget::text::body(fl!("in-use")),
                                widget::text::heading(format!(
                                    "{} ({:.1}%)",
                                    humansize::format_size(mem.swap_used, humansize::BINARY),
                                    100.0 * (mem.swap_used as f64) / (mem.swap_total as f64)
                                ))
                            ),
                        )
                        .spacing(space_m),
                        canvas(Graph::new(GraphKind::Swap, &self.graph_history).legend())
                            .height(LARGE_GRAPH_HEIGHT)
                            .width(Length::Fill),
                    )
                    .spacing(space_xxs);
                    if size.width > MIN_GRAPH_WIDTH + space_xxl as f32 + MIN_PROCESSES_WIDTH {
                        widget::row!(graph, widget::space().width(MIN_PROCESSES_WIDTH))
                            .spacing(space_xxl)
                            .into()
                    } else {
                        graph.into()
                    }
                }));

                column.into()
            }
            (NavPage::Gpu, Some(graph_item)) => {
                if let Some((gpu_i, gpu)) = graph_item
                    .gpus
                    .iter()
                    .enumerate()
                    .find(|(_, gpu)| {
                        self.gpu_id_opt == Some(gpu.id)
                            || (self.gpu_id_opt.is_none() && gpu.boot_vga)
                    })
                    .or_else(|| graph_item.gpus.first().map(|gpu| (0, gpu)))
                {
                    page_header = page_header
                        .push(
                            widget::column!(
                                widget::divider::horizontal::default(),
                                widget::dropdown(&self.gpu_names, Some(gpu_i), Message::GpuSelect,),
                                widget::divider::horizontal::default(),
                            )
                            .spacing(space_xxs),
                        )
                        .push(widget::space().height(space_m));
                    let mut column = widget::column::with_capacity(2).spacing(space_l);
                    if let Some(usage) = gpu.usage {
                        // GPU utilization and top processes
                        column = column.push(self.responsive_graph_top_processes(
                            ProcessCategory::GpuUsage(gpu.id, Some(gpu_i)),
                            move || {
                                widget::column!(
                                    widget::text::title4(fl!("gpu-utilization")),
                                    widget::row!(
                                        widget::column!(
                                            widget::text::body(fl!("utilization")),
                                            widget::text::heading(format!("{:.1}%", usage))
                                        ),
                                        if let Some(temp) = gpu.temp {
                                            widget::column!(
                                                widget::text::body(fl!("temperature")),
                                                widget::text::heading(format!("{:.1}°C", temp))
                                            )
                                        } else {
                                            widget::column!()
                                        }
                                    )
                                    .spacing(space_m),
                                    canvas(
                                        Graph::new(
                                            GraphKind::GpuUsage(gpu.id),
                                            &self.graph_history
                                        )
                                        .legend(),
                                    )
                                    .height(LARGE_GRAPH_HEIGHT)
                                    .width(Length::Fill),
                                )
                                .spacing(space_xxs)
                                .into()
                            },
                        ));
                    }
                    if let Some(vram_used) = gpu.vram_used {
                        if let Some(vram_total) = gpu.vram_total {
                            // GPU VRAM and top processes
                            column = column.push(self.responsive_graph_top_processes(
                                ProcessCategory::GpuVram(gpu.id, Some(gpu_i)),
                                move || {
                                    widget::column!(
                                        widget::text::title4(fl!("gpu-vram")),
                                        widget::row!(
                                            widget::column!(
                                                widget::text::body(fl!("capacity")),
                                                widget::text::heading(
                                                    humansize::format_size(
                                                        vram_total,
                                                        humansize::BINARY
                                                    )
                                                    .to_string()
                                                )
                                            ),
                                            widget::column!(
                                                widget::text::body(fl!("vram")),
                                                widget::text::heading(format!(
                                                    "{} ({:.1}%)",
                                                    humansize::format_size(
                                                        vram_used,
                                                        humansize::BINARY
                                                    ),
                                                    100.0 * (vram_used as f64)
                                                        / (vram_total as f64)
                                                ))
                                            ),
                                        )
                                        .spacing(space_m),
                                        canvas(
                                            Graph::new(
                                                GraphKind::GpuVram(gpu.id),
                                                &self.graph_history
                                            )
                                            .legend(),
                                        )
                                        .height(LARGE_GRAPH_HEIGHT)
                                        .width(Length::Fill),
                                    )
                                    .spacing(space_xxs)
                                    .into()
                                },
                            ));
                        }
                    }
                    column.into()
                } else {
                    widget::text::body(fl!("no-gpus")).into()
                }
            }
            (NavPage::Disk, Some(graph_item)) => {
                let mut column = widget::column::with_capacity(1 + graph_item.disks.len())
                    .spacing(space_l)
                    .width(Length::Fill);

                let all_used = graph_item.disks.iter().fold(0, |x, disk| x + disk.used);
                let all_total = graph_item.disks.iter().fold(0, |x, disk| x + disk.total);
                let all_io = graph_item.total_disk_io();
                column = column.push(self.responsive_graph_top_processes(
                    ProcessCategory::DiskTotal,
                    move || {
                        widget::column!(
                            widget::text::title4(fl!("all-disks")),
                            widget::row!(
                                widget::column!(
                                    widget::text::body(fl!("capacity")),
                                    widget::text::heading(
                                        humansize::format_size(all_total, humansize::BINARY)
                                            .to_string()
                                    )
                                ),
                                widget::column!(
                                    widget::text::body(fl!("in-use")),
                                    widget::text::heading(format!(
                                        "{} ({:.1}%)",
                                        humansize::format_size(all_used, humansize::BINARY),
                                        100.0 * (all_used as f64) / (all_total as f64)
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("reading")),
                                    widget::text::heading(format!(
                                        "{}/s",
                                        humansize::format_size(all_io.0 as u64, humansize::DECIMAL)
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("writing")),
                                    widget::text::heading(format!(
                                        "{}/s",
                                        humansize::format_size(all_io.1 as u64, humansize::DECIMAL)
                                    ))
                                ),
                            )
                            .spacing(space_m),
                            canvas(Graph::new(GraphKind::DiskTotal, &self.graph_history).legend())
                                .height(LARGE_GRAPH_HEIGHT)
                                .width(Length::Fill),
                        )
                        .spacing(space_xxs)
                        .into()
                    },
                ));

                for disk in graph_item.disks.iter() {
                    column = column.push(
                        widget::column!(
                            widget::text::title4(&disk.name),
                            widget::row!(
                                widget::column!(
                                    widget::text::body(fl!("mount-path")),
                                    widget::text::heading(&disk.mount_path)
                                ),
                                widget::column!(
                                    widget::text::body(fl!("capacity")),
                                    widget::text::heading(
                                        humansize::format_size(disk.total, humansize::BINARY)
                                            .to_string()
                                    )
                                ),
                                widget::column!(
                                    widget::text::body(fl!("in-use")),
                                    widget::text::heading(format!(
                                        "{} ({:.1}%)",
                                        humansize::format_size(disk.used, humansize::BINARY),
                                        100.0 * (disk.used as f64) / (disk.total as f64)
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("reading")),
                                    widget::text::heading(format!(
                                        "{}/s",
                                        humansize::format_size(
                                            disk.read as u64,
                                            humansize::DECIMAL
                                        )
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("writing")),
                                    widget::text::heading(format!(
                                        "{}/s",
                                        humansize::format_size(
                                            disk.write as u64,
                                            humansize::DECIMAL
                                        )
                                    ))
                                ),
                                if let Some(temp) = disk.temp {
                                    widget::column!(
                                        widget::text::body(fl!("temperature")),
                                        widget::text::heading(format!("{:.1}°C", temp))
                                    )
                                } else {
                                    widget::column!()
                                }
                            )
                            .spacing(space_m),
                            widget::responsive(move |size| {
                                let mut graphs = Vec::with_capacity(2);
                                for (title, graph_kind) in [
                                    (fl!("reading"), GraphKind::DiskRead(&disk.name)),
                                    (fl!("writing"), GraphKind::DiskWrite(&disk.name)),
                                ] {
                                    graphs.push(Element::from(
                                        widget::column!(
                                            widget::text::title4(title),
                                            canvas(
                                                Graph::new(graph_kind, &self.graph_history)
                                                    .legend(),
                                            )
                                            .height(LARGE_GRAPH_HEIGHT)
                                            .width(Length::Fill)
                                        )
                                        .spacing(space_xxs),
                                    ));
                                }
                                if size.width > MIN_GRAPH_WIDTH + space_xxl as f32 + MIN_GRAPH_WIDTH
                                {
                                    Element::from(widget::row(graphs).spacing(space_xxl))
                                } else {
                                    Element::from(widget::column(graphs).spacing(space_xxs))
                                }
                            })
                        )
                        .spacing(space_xxs),
                    );
                }
                column.into()
            }
            (NavPage::Network, Some(graph_item)) => {
                let mut column = widget::column::with_capacity(1 + graph_item.networks.len())
                    .spacing(space_l)
                    .width(Length::Fill);

                let all_io = graph_item.total_network_io();
                column = column.push(
                    widget::column!(
                        widget::text::title4(fl!("all-networks")),
                        widget::row!(
                            widget::column!(
                                widget::text::body(fl!("receiving")),
                                widget::text::heading(format!(
                                    "{}/s",
                                    humansize::format_size(all_io.0 as u64, humansize::DECIMAL)
                                ))
                            ),
                            widget::column!(
                                widget::text::body(fl!("sending")),
                                widget::text::heading(format!(
                                    "{}/s",
                                    humansize::format_size(all_io.1 as u64, humansize::DECIMAL)
                                ))
                            ),
                        )
                        .spacing(space_m),
                        canvas(Graph::new(GraphKind::NetworkTotal, &self.graph_history).legend())
                            .height(LARGE_GRAPH_HEIGHT)
                            .width(Length::Fill),
                    )
                    .spacing(space_xxs),
                );

                for net in graph_item.networks.iter() {
                    column = column.push(
                        widget::column!(
                            widget::text::title4(&net.name),
                            widget::row!(
                                widget::column!(
                                    widget::text::body(fl!("receiving")),
                                    widget::text::heading(format!(
                                        "{}/s",
                                        humansize::format_size(net.rx as u64, humansize::DECIMAL)
                                    ))
                                ),
                                widget::column!(
                                    widget::text::body(fl!("sending")),
                                    widget::text::heading(format!(
                                        "{}/s",
                                        humansize::format_size(net.tx as u64, humansize::DECIMAL)
                                    ))
                                ),
                            )
                            .spacing(space_m),
                            widget::responsive(move |size| {
                                let mut graphs = Vec::with_capacity(2);
                                for (title, graph_kind) in [
                                    (fl!("receiving"), GraphKind::NetworkRx(&net.name)),
                                    (fl!("sending"), GraphKind::NetworkTx(&net.name)),
                                ] {
                                    graphs.push(Element::from(
                                        widget::column!(
                                            widget::text::title4(title),
                                            canvas(
                                                Graph::new(graph_kind, &self.graph_history)
                                                    .legend(),
                                            )
                                            .height(LARGE_GRAPH_HEIGHT)
                                            .width(Length::Fill)
                                        )
                                        .spacing(space_xxs),
                                    ));
                                }
                                if size.width > 800.0 {
                                    Element::from(widget::row(graphs))
                                } else {
                                    Element::from(widget::column(graphs))
                                }
                            })
                        )
                        .spacing(space_xxs),
                    );
                }
                column.into()
            }
            _ => widget::indeterminate_circular().into(),
        };
        let content = widget::mouse_area(
            widget::column!(
                page_header,
                widget::scrollable(
                    widget::container(content)
                        .padding([0, space_xl, space_s, space_xl])
                        .width(Length::Fill)
                )
                .width(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .on_press(Message::ProcessSelect(None));
        if let Some(id) = self.nav_model.active_data::<widget::Id>() {
            widget::id_container(content, id.clone()).into()
        } else {
            content.into()
        }
    }

    fn system_theme_update(
        &mut self,
        _keys: &[&'static str],
        _new_theme: &cosmic::cosmic_theme::Theme,
    ) -> Task<Self::Message> {
        self.update(Message::SystemThemeChange)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        struct ConfigSubscription;

        Subscription::batch([
            Subscription::run(info::worker),
            cosmic_config::config_subscription(
                TypeId::of::<ConfigSubscription>(),
                Self::APP_ID.into(),
                CONFIG_VERSION,
            )
            .map(|update| {
                if !update.errors.is_empty() {
                    log::debug!(
                        "errors loading config {:?}: {:?}",
                        update.keys,
                        update.errors
                    );
                }
                Message::Config(Box::new(update.config))
            }),
        ])
    }
}

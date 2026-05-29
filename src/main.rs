// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    Application, Element,
    app::{Core, Settings, Task, context_drawer},
    cosmic_config::{self, CosmicConfigEntry},
    cosmic_theme, executor,
    iced::{
        self, Alignment, Length, Limits, Subscription,
        core::text::{Ellipsize, EllipsizeHeightLimit, Shaping},
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
use std::{
    any::TypeId,
    collections::{HashMap, VecDeque},
    env,
    error::Error,
    time::{Duration, Instant},
};

use config::{AppTheme, CONFIG_VERSION, Config};
mod config;

use graph::{Graph, GraphKind};
mod graph;

use info::{GraphItem, ProcessCategory, ProcessItem};
mod info;

mod localize;

use menu::menu_bar;
mod menu;

use clap_lex::RawArgs;

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
            _ => {
                //TODO: should this throw an error?
                log::warn!("ignored argument {:?}", arg);
            }
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

/// Messages that are used specifically by our [`App`].
#[derive(Clone, Debug)]
pub enum Message {
    None,
    AppTheme(AppTheme),
    Config(Box<Config>),
    Graph(GraphItem),
    LaunchUrl(String),
    NavPage(NavPage),
    ProcessSort(ProcessCategory),
    Snapshot(GraphItem, Vec<ProcessItem>),
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
    config: Config,
    config_handler: Option<cosmic_config::Config>,
    context_page: ContextPage,
    core: Core,
    graph_history: VecDeque<GraphItem>,
    graph_snapshot: Option<GraphItem>,
    key_binds: HashMap<KeyBind, Action>,
    nav_model: segmented_button::SingleSelectModel,
    processes: Vec<ProcessItem>,
    process_content: iced::widget::list::Content<ProcessItem>,
    process_sort: (ProcessCategory, bool),
}

impl App {
    fn update_config(&mut self) -> Task<Message> {
        let theme = self.config.app_theme.theme();
        cosmic::command::set_theme(theme)
    }

    fn update_processes(&mut self) {
        self.processes.sort_by(|a, b| {
            if self.process_sort.1 {
                b.compare(a, self.process_sort.0)
            } else {
                a.compare(b, self.process_sort.0)
            }
        });

        let mut i = 0;
        for process in self.processes.iter() {
            if i >= self.process_content.len() {
                self.process_content.push(process.clone());
            } else if self.process_content.get(i) != Some(&process) {
                *self.process_content.get_mut(i).unwrap() = process.clone();
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
                b.text(page.title()).data::<NavPage>(page)
            });
        }

        let mut app = Self {
            about,
            app_themes,
            config: flags.config,
            config_handler: flags.config_handler,
            context_page: ContextPage::Settings,
            core,
            graph_history: VecDeque::new(),
            graph_snapshot: None,
            key_binds: HashMap::new(),
            nav_model: nav_model.build(),
            processes: Vec::new(),
            process_content: iced::widget::list::Content::new(),
            process_sort: (ProcessCategory::CPU, false),
        };

        let command = app.update_config();
        (app, command)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav_model)
    }

    //TODO: currently the first escape unfocuses, and the second calls this function
    fn on_escape(&mut self) -> Task<Message> {
        if self.core.window.show_context {
            return self.update(Message::ToggleContextPage(self.context_page));
        }
        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Self::Message> {
        self.nav_model.activate(id);
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
                    return self.on_nav_select(id);
                }
            }
            Message::ProcessSort(category) => {
                if self.process_sort.0 == category {
                    self.process_sort.1 = !self.process_sort.1
                } else {
                    self.process_sort.0 = category;
                    self.process_sort.1 = false;
                }
                self.update_processes();
            }
            Message::Snapshot(graph_item, processes) => {
                self.graph_snapshot = Some(graph_item);
                self.processes = processes;
                self.update_processes();
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

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![menu_bar(&self.core, &self.config, &self.key_binds)]
    }

    /// Creates a view after each update.
    fn view(&self) -> Element<'_, Self::Message> {
        let cosmic_theme::Spacing {
            space_m,
            space_s,
            space_xs,
            space_xxs,
            ..
        } = theme::active().cosmic().spacing;

        let nav_page = self
            .nav_model
            .active_data()
            .map_or(NavPage::Dashboard, |x| *x);
        let content: Element<Message> = match (nav_page, &self.graph_snapshot) {
            (NavPage::Dashboard, Some(graph_item)) => {
                let card = |graph_kind,
                            graph_left,
                            graph_right,
                            name,
                            caption,
                            process_category: Option<ProcessCategory>,
                            nav_page: NavPage|
                 -> Element<Message> {
                    let mut column = widget::column::with_capacity(7)
                        .spacing(space_xxs)
                        .push(
                            canvas(Graph::new(graph_kind, &self.graph_history).border())
                                .height(176.0)
                                .width(Length::Fill),
                        )
                        .push(widget::row!(
                            widget::text::body(graph_left).width(Length::Fill),
                            widget::text::body(graph_right).width(Length::Fill),
                        ))
                        .push(widget::column!(
                            widget::text::body(name),
                            widget::text::caption(caption)
                                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                        ));

                    if let Some(sort_category) = process_category {
                        // The compare function is backwards, so this uses min_by
                        if let Some(process) = self
                            .processes
                            .iter()
                            .min_by(|a, b| a.compare(b, sort_category))
                        {
                            let mut row = widget::row::with_capacity(2).align_y(Alignment::Center);
                            for &category in &[ProcessCategory::Name, sort_category] {
                                row = row.push(
                                    widget::container(
                                        widget::text(process.text(category))
                                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(
                                                1,
                                            )))
                                            .shaping(Shaping::Basic),
                                    )
                                    .align_x(category.data_align())
                                    .align_y(Alignment::Center)
                                    .width(category.width()),
                                );
                            }
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
                                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(
                                                1,
                                            )))
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
                            .on_press(Message::NavPage(nav_page)),
                    );

                    widget::container(column)
                        .class(theme::Container::Card)
                        .padding([space_xxs, space_s])
                        .width(300.0)
                        .into()
                };

                let mut flex_row = Vec::with_capacity(2 + graph_item.gpus.len());
                flex_row.push(card(
                    GraphKind::Cpu,
                    format!("{:.1}%", graph_item.total_cpu_usage()),
                    String::new(),
                    fl!("cpu"),
                    graph_item
                        .cpus
                        .first()
                        .map(|x| x.brand.clone())
                        .unwrap_or_default(),
                    Some(ProcessCategory::CPU),
                    NavPage::Cpu,
                ));

                flex_row.push(card(
                    GraphKind::Memory,
                    format!(
                        "{:.1}%",
                        100.0 * (graph_item.memory.used as f32) / (graph_item.memory.total as f32),
                    ),
                    format!(
                        "{}",
                        humansize::format_size(graph_item.memory.used, humansize::BINARY),
                    ),
                    fl!("memory"),
                    format!(
                        "{}",
                        humansize::format_size(graph_item.memory.total, humansize::BINARY),
                    ),
                    Some(ProcessCategory::Memory),
                    NavPage::Memory,
                ));

                let disk_io = graph_item.total_disk_io();
                flex_row.push(card(
                    GraphKind::DiskTotal,
                    format!(
                        "{}/s read",
                        humansize::format_size(disk_io.0 as u64, humansize::DECIMAL),
                    ),
                    format!(
                        "{}/s write",
                        humansize::format_size(disk_io.1 as u64, humansize::DECIMAL)
                    ),
                    fl!("disk"),
                    String::new(),
                    Some(ProcessCategory::DiskTotal),
                    NavPage::Disk,
                ));

                let network_io = graph_item.total_network_io();
                flex_row.push(card(
                    GraphKind::NetworkTotal,
                    format!(
                        "{}/s rx",
                        humansize::format_size(network_io.0 as u64, humansize::DECIMAL),
                    ),
                    format!(
                        "{}/s tx",
                        humansize::format_size(network_io.1 as u64, humansize::DECIMAL)
                    ),
                    fl!("network"),
                    String::new(),
                    None,
                    NavPage::Network,
                ));

                for gpu in graph_item.gpus.iter() {
                    let Some(usage) = gpu.usage else { continue };

                    flex_row.push(card(
                        GraphKind::GpuUsage(&gpu.bus_id),
                        format!("{:.1}%", usage),
                        String::new(),
                        fl!("gpu"),
                        gpu.name.clone(),
                        //TODO: filter by GPU
                        Some(ProcessCategory::GPU),
                        NavPage::Gpu,
                    ));
                }

                widget::flex_row(flex_row).spacing(space_xxs).into()
            }
            (NavPage::Processes, _) => {
                //TODO: table is too slow, this uses list to emulate table
                let categories = ProcessCategory::all();
                let mut header =
                    widget::row::with_capacity(categories.len()).align_y(Alignment::Center);
                for &category in categories {
                    let mut row = widget::row::with_capacity(2)
                        .align_y(Alignment::Center)
                        .height(Length::Fixed(24.0))
                        .padding([0, 8])
                        .width(category.width());
                    row = row.push(widget::text::heading(category.to_string()));
                    if category == self.process_sort.0 {
                        row = row.push(
                            widget::icon::from_name(if self.process_sort.1 {
                                "pan-up-symbolic"
                            } else {
                                "pan-down-symbolic"
                            })
                            .size(16),
                        );
                    }
                    header = header
                        .push(widget::mouse_area(row).on_press(Message::ProcessSort(category)));
                }
                widget::column::with_capacity(2)
                    .push(header)
                    .push(iced::widget::List::new(
                        &self.process_content,
                        move |_i, item| {
                            let mut row = widget::row::with_capacity(categories.len())
                                .align_y(Alignment::Center);
                            for &category in categories {
                                row = row.push(
                                    widget::container(
                                        widget::text(item.text(category))
                                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(
                                                1,
                                            )))
                                            .shaping(Shaping::Basic),
                                    )
                                    .align_x(category.data_align())
                                    .align_y(Alignment::Center)
                                    .padding([0, 8])
                                    .height(Length::Fixed(40.0))
                                    .width(category.width()),
                                );
                            }
                            widget::column::with_capacity(2)
                                .push(widget::divider::horizontal::default())
                                .push(row)
                                .into()
                        },
                    ))
                    .width(Length::Fill)
                    .into()
            }
            (NavPage::Cpu, Some(graph_item)) => {
                let mut column = widget::column::with_capacity(2)
                    .spacing(space_m)
                    .width(Length::Fill);

                // Overall utilization
                column = column.push(
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
                                widget::text::heading("TODO GHz")
                            ),
                            widget::column!(
                                widget::text::body(fl!("temperature")),
                                widget::text::heading("TODO °C")
                            ),
                        )
                        .spacing(space_m),
                        canvas(Graph::new(GraphKind::Cpu, &self.graph_history).legend())
                            .height(300.0)
                            .width(Length::Fill),
                    )
                    .spacing(space_xxs),
                );

                // Utilization per core
                let mut children = Vec::with_capacity(graph_item.cpus.len());
                for cpu in graph_item.cpus.iter() {
                    let mut row = widget::row::with_capacity(2).align_y(Alignment::Center);
                    row = row.push(
                        widget::determinate_linear(cpu.usage / 100.0)
                            .girth(12.0)
                            .width(240.0),
                    );
                    row = row.push(
                        widget::text(format!("{:.1}%", cpu.usage))
                            .align_x(Alignment::End)
                            .width(48.0),
                    );
                    children.push(widget::column!(widget::text::heading(&cpu.name), row).into());
                }
                column = column.push(
                    widget::column!(
                        widget::text::title4(fl!("utilization-per-core")),
                        widget::flex_row(children)
                            .column_spacing(space_s)
                            .row_spacing(space_xs)
                    )
                    .spacing(space_xxs),
                );

                column.into()
            }
            (NavPage::Memory, Some(graph_item)) => {
                let mem = &graph_item.memory;

                let mut column = widget::column::with_capacity(2)
                    .spacing(space_m)
                    .width(Length::Fill);

                // Memory information
                column = column.push(
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
                                widget::text::heading("TODO")
                            ),
                            widget::column!(
                                widget::text::body(fl!("total-utilization")),
                                widget::text::heading("TODO")
                            ),
                        )
                        .spacing(space_m),
                        canvas(Graph::new(GraphKind::Memory, &self.graph_history).legend())
                            .height(300.0)
                            .width(Length::Fill),
                    )
                    .spacing(space_xxs),
                );

                // Swap information
                column = column.push(
                    widget::column!(
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
                            .height(300.0)
                            .width(Length::Fill),
                    )
                    .spacing(space_xxs),
                );

                column.into()
            }
            (NavPage::Gpu, Some(graph_item)) => {
                let mut column = widget::column::with_capacity(graph_item.gpus.len())
                    .spacing(space_m)
                    .width(Length::Fill);

                for gpu in graph_item.gpus.iter() {
                    let mut gpu_col = widget::column::with_capacity(5).spacing(space_xxs);
                    gpu_col = gpu_col.push(widget::text::title4(&gpu.name));
                    if let Some(usage) = gpu.usage {
                        gpu_col = gpu_col.push(
                            widget::row!(widget::column!(
                                widget::text::body(fl!("utilization")),
                                widget::text::heading(format!("{:.1}%", usage))
                            ),)
                            .spacing(space_m),
                        );
                        gpu_col = gpu_col.push(
                            canvas(
                                Graph::new(GraphKind::GpuUsage(&gpu.bus_id), &self.graph_history)
                                    .legend(),
                            )
                            .height(300.0)
                            .width(Length::Fill),
                        );
                    }
                    if let Some(vram_used) = gpu.vram_used {
                        if let Some(vram_total) = gpu.vram_total {
                            gpu_col = gpu_col.push(
                                widget::row!(
                                    widget::column!(
                                        widget::text::body(fl!("capacity")),
                                        widget::text::heading(
                                            humansize::format_size(vram_total, humansize::BINARY)
                                                .to_string()
                                        )
                                    ),
                                    widget::column!(
                                        widget::text::body(fl!("vram")),
                                        widget::text::heading(format!(
                                            "{} ({:.1}%)",
                                            humansize::format_size(vram_used, humansize::BINARY),
                                            100.0 * (vram_used as f64) / (vram_total as f64)
                                        ))
                                    ),
                                )
                                .spacing(space_m),
                            );
                            gpu_col = gpu_col.push(
                                canvas(
                                    Graph::new(
                                        GraphKind::GpuVram(&gpu.bus_id),
                                        &self.graph_history,
                                    )
                                    .legend(),
                                )
                                .height(300.0)
                                .width(Length::Fill),
                            );
                        }
                    }
                    column = column.push(gpu_col);
                }

                column.into()
            }
            (NavPage::Disk, Some(graph_item)) => {
                let mut column =
                    widget::column::with_capacity(graph_item.disks.len() * 8).width(Length::Fill);
                for disk in graph_item.disks.iter() {
                    column = column.push(widget::text(format!("Name: {}", disk.name)));
                    column = column.push(widget::text(format!(
                        "Used: {} ({:.1}%)",
                        humansize::format_size(disk.used, humansize::DECIMAL),
                        100.0 * (disk.used as f64) / (disk.total as f64)
                    )));
                    column = column.push(widget::text(format!(
                        "Total: {}",
                        humansize::format_size(disk.total, humansize::DECIMAL)
                    )));
                    column = column.push(widget::text(format!(
                        "Read: {}/s",
                        humansize::format_size(disk.read as u64, humansize::DECIMAL)
                    )));
                    column = column.push(
                        canvas(
                            Graph::new(GraphKind::DiskRead(&disk.name), &self.graph_history)
                                .legend(),
                        )
                        .height(300.0)
                        .width(Length::Fill),
                    );
                    column = column.push(widget::text(format!(
                        "Write: {}/s",
                        humansize::format_size(disk.write as u64, humansize::DECIMAL)
                    )));
                    column = column.push(
                        canvas(
                            Graph::new(GraphKind::DiskWrite(&disk.name), &self.graph_history)
                                .legend(),
                        )
                        .height(300.0)
                        .width(Length::Fill),
                    );
                    column = column.push(widget::space().height(20.0));
                }
                column.into()
            }
            (NavPage::Network, Some(graph_item)) => {
                let mut column = widget::column::with_capacity(graph_item.networks.len() * 6)
                    .width(Length::Fill);
                for net in graph_item.networks.iter() {
                    column = column.push(widget::text(format!("Name: {}", net.name)));
                    column = column.push(widget::text(format!(
                        "Rx: {}/s",
                        humansize::format_size(net.rx as u64, humansize::DECIMAL)
                    )));
                    column = column.push(
                        canvas(
                            Graph::new(GraphKind::NetworkRx(&net.name), &self.graph_history)
                                .legend(),
                        )
                        .height(300.0)
                        .width(Length::Fill),
                    );
                    column = column.push(widget::text(format!(
                        "Tx: {}/s",
                        humansize::format_size(net.tx as u64, humansize::DECIMAL)
                    )));
                    column = column.push(
                        canvas(
                            Graph::new(GraphKind::NetworkTx(&net.name), &self.graph_history)
                                .legend(),
                        )
                        .height(300.0)
                        .width(Length::Fill),
                    );
                    column = column.push(widget::space().height(20.0));
                }
                column.into()
            }
            _ => widget::indeterminate_circular().into(),
        };
        widget::scrollable(
            widget::column::with_capacity(2)
                .spacing(space_m)
                .width(Length::Fill)
                .push(if matches!(nav_page, NavPage::Dashboard) {
                    widget::column!(widget::text::title2(nav_page.title()))
                } else {
                    widget::column!(
                        widget::button::text(fl!("dashboard"))
                            .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                            .on_press(Message::NavPage(NavPage::Dashboard)),
                        widget::text::title2(nav_page.title())
                    )
                })
                .push(content),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
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

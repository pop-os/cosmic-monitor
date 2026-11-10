use std::{cmp::Ordering, sync::Arc};

use freedesktop_desktop_entry::{DesktopEntry, Iter, default_paths, get_languages_from_env};
use sysinfo::{Process, System};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppEntry {
    pub id: String,
    pub icon: Option<String>,
    pub name: Option<String>,
    pub no_display: bool,
    pub args: Vec<String>,
}

impl AppEntry {
    pub fn get_entries() -> Vec<Arc<AppEntry>> {
        let locales = get_languages_from_env();
        let mut app_entries = Vec::new();
        for app in Iter::new(default_paths())
            .filter_map(|p| DesktopEntry::from_path(p, Some(&locales)).ok())
        {
            let Ok(args) = app.parse_exec() else { continue };
            let id = app.id().to_string();
            let mut icon = app.icon().map(|x| x.to_string());

            // Fixup for firefox user app icon
            if icon.is_none() && id.starts_with("userapp-Firefox-") {
                icon = Some("firefox".to_string());
            }

            app_entries.push(Arc::new(AppEntry {
                id,
                icon,
                name: app.full_name(&locales).map(|x| x.to_string()),
                no_display: app.no_display(),
                args,
            }));
        }

        // Sort no_display below
        app_entries.sort_unstable_by(|a, b| match (a.no_display, b.no_display) {
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        app_entries
    }

    pub fn process_app<'a>(
        entries: &[Arc<Self>],
        mut process: &'a Process,
        sys: &'a System,
    ) -> Option<Arc<Self>> {
        // This loops to look for any parent processes that have an associated app, as well
        //TODO: maximum depth for parent app search?
        loop {
            let proc_args = process.cmd();

            // Handle flatpaks
            #[cfg(target_os = "linux")]
            if let Ok(Some(name)) = freedesktop_desktop_entry::group_entry_from_path(
                format!("/proc/{}/root/.flatpak-info", process.pid()),
                "Application",
                "name",
            ) {
                for app in entries.iter() {
                    if app.id == name {
                        return Some(app.clone());
                    }
                }
            }

            let proc_cmd = proc_args.first().and_then(|x| x.to_str())?;
            let proc_exe = process.exe().and_then(|x| x.to_str())?;
            for app in entries.iter() {
                let Some(cmd) = app.args.first() else {
                    continue;
                };
                if proc_cmd == cmd || proc_exe == cmd {
                    return Some(app.clone());
                }
            }
            let parent = process.parent()?;
            process = sys.process(parent)?;
        }
    }
}

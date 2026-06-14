#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppEntry {
    pub id: String,
    pub icon: Option<String>,
    pub name: Option<String>,
    pub no_display: bool,
    pub args: Vec<String>,
}

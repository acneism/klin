#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddonAction {
    pub title: String,
    pub badge_text: String,
    pub badge_color: [u8; 4],
    pub has_popup: bool,
    pub enabled: bool,
    pub icon: Option<std::sync::Arc<crate::Image>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddonInfo {
    pub id: String,
    pub uuid: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub has_options: bool,
    pub warnings: Vec<String>,
    pub action: Option<AddonAction>,
}

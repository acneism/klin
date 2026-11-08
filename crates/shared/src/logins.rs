#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedLogin {
    pub id: String,
    pub origin: String,
    pub form_action_origin: Option<String>,
    pub http_realm: Option<String>,
    pub username: String,
    pub password: String,
    pub time_created: i64,
    pub time_last_used: i64,
    pub time_password_changed: i64,
    pub times_used: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoginInput {
    pub origin: String,
    pub form_action_origin: Option<String>,
    pub http_realm: Option<String>,
    pub username_field: String,
    pub password_field: String,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportedLogin {
    pub input: LoginInput,
    pub guid: Option<String>,
    pub time_created: i64,
    pub time_last_used: i64,
    pub time_password_changed: i64,
    pub times_used: i64,
}

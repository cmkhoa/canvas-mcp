//! Slim views of Canvas objects: only the fields the tools use. Serde ignores the rest.
use serde::{Deserialize, Serialize};

/// `GET /users/self/profile`. Email is deliberately not modeled.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub login_id: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
}

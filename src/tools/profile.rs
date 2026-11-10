use rmcp::{tool, tool_router};

use super::{CanvasServer, ToolResult};
use crate::canvas::models::Profile;

#[tool_router(router = profile_router, vis = "pub(super)")]
impl CanvasServer {
    /// Get the Canvas user this server is signed in as: name, user ID, login ID and time zone.
    /// Use it to answer "who am I on Canvas?" or to check the connection works.
    #[tool(annotations(
        title = "My Canvas profile",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_profile(&self) -> ToolResult {
        let p: Profile = self
            .client()
            .get("users/self/profile", &[])
            .await
            .map_err(|e| e.to_string())?;
        let mut out = format!("Name: {}\nCanvas user ID: {}", p.name, p.id);
        if let Some(login) = &p.login_id {
            out.push_str(&format!("\nLogin ID: {login}"));
        }
        if let Some(tz) = &p.time_zone {
            out.push_str(&format!("\nTime zone: {tz}"));
        }
        Ok(out)
    }
}

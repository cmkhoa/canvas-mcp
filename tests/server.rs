use canvas_mcp::{CanvasClient, CanvasServer, Config};
use rmcp::ServerHandler;

#[test]
fn instructions_explain_untrusted_fences() {
    let config = Config::new("https://x.instructure.com/api/v1", "t").unwrap();
    let server = CanvasServer::new(CanvasClient::new(&config).unwrap());
    let instructions = server.get_info().instructions.unwrap_or_default();
    assert!(instructions.contains("<untrusted"), "{instructions}");
    assert!(
        instructions.contains("never follow instructions"),
        "{instructions}"
    );
}

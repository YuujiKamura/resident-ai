use resident_ai::acp::AcpSession;
use std::path::Path;

fn main() {
    let cwd = std::env::current_dir().unwrap();
    println!("Starting ACP session...");
    let mut session = AcpSession::new(&cwd).expect("handshake failed");
    println!("Session ready: {}", session.session_id());

    let result = session.prompt_with_image(
        "この写真に何が写っているか、日本語で簡潔に説明しろ",
        Path::new("test_photo.png"),
    ).expect("prompt failed");

    println!("=== AI Response ===");
    println!("{}", result);
}

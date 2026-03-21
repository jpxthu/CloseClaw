//! CloseClaw Binary Entry Point

use closeclaw::init;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init();
    
    println!("CloseClaw v{} - Lightweight, rule-driven multi-agent framework", env!("CARGO_PKG_VERSION"));
    println!("Type 'help' for available commands.");
    
    // TODO: Implement CLI loop
    // For now, just demonstrate initialization
    
    Ok(())
}

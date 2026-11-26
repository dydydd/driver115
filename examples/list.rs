use driver115::{driver, Client};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: list <config.yaml>");
    let client = Client::from_yaml(&path)?;
    let dirs = driver::list_dirs(&client, "0").await?;
    println!("根目录 {} 项", dirs.len());
    for d in dirs.iter().take(20) {
        println!("  {}  {}", d.id, d.name);
    }
    Ok(())
}

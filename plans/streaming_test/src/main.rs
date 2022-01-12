mod synchronization;

use redis::Client;

use synchronization::Barrier;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::open("redis://127.0.0.1/")?;

    let barrier = Barrier::new(&client, "state", 5).await?;

    if let Err(e) = barrier.wait().await {
        eprintln!("{}", e);
    }

    Ok(())
}

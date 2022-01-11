use redis::{AsyncCommands, Client};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::open("redis://127.0.0.1/").unwrap();

    let mut con = client.get_tokio_connection().await.unwrap();

    con.set("key1", b"foo").await?;

    Ok(())
}

mod ipfs;
mod roles;
mod utils;

use roles::{streamer::*, viewer::*};

use testground::client::Client;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub const REDIS_TOPIC: &str = "addr_ex";
pub const INIT_STATE: &str = "init";
pub const NET_STATE: &str = "net";
pub const BOOTSTRAP_STATE: &str = "bootstrapped";
pub const STOP_STATE: &str = "stop";
pub const GOSSIPSUB_TOPIC: &str = "live";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let (testground, args) = Client::new().await?;

    testground.wait_network_initialized().await?;

    let sim_id = testground.signal("id").await? as usize;

    if sim_id == 1 {
        streamer(sim_id, testground, args).await
    /*  } else if sim_id <= args.test_instance_count / 4 {
    neutral(sim_id, testground, args).await */
    } else {
        viewer(sim_id, testground, args).await
    }
}

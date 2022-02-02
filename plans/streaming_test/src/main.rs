mod ipfs;
mod nodes;
mod utils;

use nodes::{neutral::*, streamer::*, viewer::*};
use utils::*;

use std::path::PathBuf;

use structopt::StructOpt;

use pnet::ipnetwork::IpNetwork;

use libp2p::{multiaddr::Protocol, Multiaddr};

use testground::client::Client;

#[derive(StructOpt)]
#[allow(dead_code)]
pub struct Arguments {
    //PATH: /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    #[structopt(env)]
    hostname: Option<String>, // HOSTNAME: e6f4cc8fc147
    #[structopt(env)]
    test_temp_path: Option<String>, // TEST_TEMP_PATH: /temp
    #[structopt(env)]
    test_plan: String, // TEST_PLAN: streaming_test
    #[structopt(env)]
    test_branch: Option<String>, // TEST_BRANCH:
    #[structopt(env)]
    test_instance_count: u64, // TEST_INSTANCE_COUNT: 1
    #[structopt(env)]
    test_instance_role: Option<String>, // TEST_INSTANCE_ROLE:
    #[structopt(env)]
    test_outputs_path: Option<PathBuf>, // TEST_OUTPUTS_PATH: /outputs
    #[structopt(env)]
    test_run: String, // TEST_RUN: c7fjstge5te621cen4i0
    #[structopt(long, env)]
    test_sidecar: bool, // TEST_SIDECAR: true
    #[structopt(env)]
    test_group_instance_count: Option<usize>, // TEST_GROUP_INSTANCE_COUNT: 1
    #[structopt(env)]
    test_group_id: String, // TEST_GROUP_ID: single
    #[structopt(env)]
    test_instance_params: Option<String>, // TEST_INSTANCE_PARAMS: feature=false|neutral_nodes=10|num=2|word=never
    #[structopt(env)]
    test_repo: Option<String>, //TEST_REPO:
    #[structopt(env)]
    test_tag: Option<String>, // TEST_TAG:
    #[structopt(env)]
    test_disable_metrics: Option<bool>, // TEST_DISABLE_METRICS: false
    #[structopt(env)]
    test_case: String, // TEST_CASE: quickstart
    #[structopt(env)]
    test_subnet: IpNetwork, // TEST_SUBNET: 16.0.0.0/16
    #[structopt(env)]
    test_capture_profiles: Option<String>, // TEST_CAPTURE_PROFILES:
    #[structopt(env)]
    test_start_time: Option<String>, // TEST_START_TIME: 2022-01-12T15:48:07-05:00
    #[structopt(env)]
    influxdb_url: Option<String>, // INFLUXDB_URL: http://testground-influxdb:8086
    #[structopt(env)]
    redis_host: String, // REDIS_HOST: testground-redis
                        // HOME: /
}

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub const REDIS_TOPIC: &str = "addr_ex";

pub const INIT_STATE: &str = "init";
pub const NET_STATE: &str = "net";
pub const BOOTSTRAP_STATE: &str = "bootstrapped";
pub const STOP_STATE: &str = "stop";

pub const SIM_TIME: u64 = 60;

pub const GOSSIPSUB_TOPIC: &str = "live";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let sync_client: Client = Client::new().await.expect("Creating sync service");

    let args = Arguments::from_args();

    let local_multi_addr = {
        let local_ip = data_network_ip(args.test_sidecar, args.test_subnet);

        let mut addr: Multiaddr = local_ip.into();

        addr.push(Protocol::Tcp(4001));

        addr
    };

    let (ipfs, handle, local_peer_id) = ipfs::compose_network();

    ipfs.listen_on(local_multi_addr.clone())
        .await
        .expect("Listening on local address");

    sync_client
        .wait_network_initialized()
        .await
        .expect("Network initialization");

    let sim_id = sync_client
        .signal(INIT_STATE)
        .await
        .expect("Waiting for signal");

    if sim_id == 1 {
        streamer(
            sim_id,
            ipfs,
            sync_client,
            handle,
            args,
            local_peer_id,
            local_multi_addr,
        )
        .await
    } else if sim_id <= args.test_instance_count / 4 {
        neutral(
            sim_id,
            ipfs,
            sync_client,
            handle,
            args,
            local_peer_id,
            local_multi_addr,
        )
        .await
    } else {
        viewer(
            sim_id,
            ipfs,
            sync_client,
            handle,
            args,
            local_peer_id,
            local_multi_addr,
        )
        .await
    }
}

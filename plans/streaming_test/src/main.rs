mod synchronization;
mod utils;

use std::{net::IpAddr, path::PathBuf};

use structopt::StructOpt;

use synchronization::*;
use utils::*;

use pnet::ipnetwork::IpNetwork;

use futures_util::stream::StreamExt;

#[derive(StructOpt)]
#[allow(dead_code)]
struct Arguments {
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
    test_instance_count: usize, // TEST_INSTANCE_COUNT: 1
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
    test_instance_params: Option<String>, // TEST_INSTANCE_PARAMS:
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

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Arguments::from_args();

    let sync_client = SyncClient::new(&args.redis_host).await?;

    sync_client
        .wait_for_network_initialized(
            args.test_run,
            args.test_plan,
            args.test_case,
            args.test_group_id,
            args.test_instance_count,
        )
        .await?;

    let ip = get_data_network_ip(args.test_sidecar, args.test_subnet);

    let mut stream = sync_client.subscribe("addresses").await?;

    let seq = sync_client.signal_entry("init").await?;

    println!("I'm intance #{} and my IP address is {:?}", seq, ip);

    let mut barrier = sync_client.barrier("init", args.test_instance_count);
    barrier.down().await?; //Barrier waiting for every container to subscribe.

    sync_client.publish("addresses", ip.to_string()).await?;

    let mut ips: Vec<IpAddr> = Vec::with_capacity(args.test_instance_count);

    sync_client.signal_entry("ready").await?;
    let mut barrier = sync_client.barrier("ready", args.test_instance_count);

    loop {
        tokio::select! {
            biased;//pool future in order. We want all the msg then check if barrier is down.

            Some(msg) = stream.next() => {
                let payload: String = msg.get_payload().unwrap();

                let ip = payload.parse().unwrap();

                ips.push(ip);
            },
            _ = barrier.down() => break,
        }
    }

    println!("{} Received IPs {:?}", ips.len(), ips);

    Ok(())
}

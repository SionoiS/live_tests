mod synchronization;
mod utils;

use std::path::PathBuf;

use structopt::StructOpt;

use synchronization::*;
use utils::*;

use pnet::ipnetwork::IpNetwork;

use futures_util::stream::StreamExt;

use libp2p::{
    identity,
    multiaddr::Protocol,
    ping::{Ping, PingConfig},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
};

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

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let args = Arguments::from_args();

    let own_ip = get_data_network_ip(args.test_sidecar, args.test_subnet);

    let sync_client = SyncClient::new(&args.redis_host).await?;

    /* sync_client
    .wait_for_network_initialized(
        args.test_sidecar,
        args.test_run,
        args.test_plan,
        args.test_case,
        args.test_group_id,
        args.test_instance_count,
    )
    .await?; */

    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());

    let transport = libp2p::development_transport(local_key).await?;

    let behaviour = Ping::new(PingConfig::new().with_keep_alive(true));

    let mut swarm = Swarm::new(transport, behaviour, local_peer_id);

    let mut own_multi_addr: Multiaddr = own_ip.into();
    own_multi_addr.push(Protocol::Tcp(8080));
    swarm.listen_on(own_multi_addr.clone())?;

    let mut stream = sync_client.subscribe("addresses").await?;

    sync_client
        .signal_and_wait("init", args.test_instance_count)
        .await?;
    //Barrier waiting for every container to subscribe.

    sync_client
        .publish("addresses", own_multi_addr.to_string())
        .await?;

    let mut addresses: Vec<Multiaddr> = Vec::with_capacity(args.test_instance_count);

    sync_client.signal_entry("ready").await?;

    loop {
        tokio::select! {
            biased;//pool future in order. We want all the msg then check if barrier is down.

            Some(msg) = stream.next() => {
                let payload: String = msg.get_payload().unwrap();

                let multi_addr = payload.parse().unwrap();

                addresses.push(multi_addr);
            },
            _ = sync_client.barrier("ready", args.test_instance_count) => break,
        }
    }

    for multi_addr in addresses {
        if multi_addr == own_multi_addr {
            continue;
        }

        println!("Dialing {:?}", multi_addr);

        swarm.dial(multi_addr)?;
    }

    let sleep = tokio::time::sleep(std::time::Duration::from_secs(60));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            Some(msg) = swarm.next() => match msg {
                SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {:?}", address),
                SwarmEvent::Behaviour(event) => println!("{:?}", event),
                _ => {}
            },
            _ = &mut sleep => break,
        }
    }

    Ok(())
}

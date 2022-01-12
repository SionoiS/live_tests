mod synchronization;

use std::path::PathBuf;

use structopt::StructOpt;

use redis::Client;

use synchronization::*;

#[derive(StructOpt)]
struct Arguments {
    //PATH: /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    #[structopt(env)]
    hostname: Option<String>, // HOSTNAME: e6f4cc8fc147
    #[structopt(env)]
    test_temp_path: Option<String>, // TEST_TEMP_PATH: /temp
    #[structopt(env)]
    test_plan: Option<String>, // TEST_PLAN: streaming_test
    #[structopt(env)]
    test_branch: Option<String>, // TEST_BRANCH:
    #[structopt(env)]
    test_instance_count: usize, // TEST_INSTANCE_COUNT: 1
    #[structopt(env)]
    test_instance_role: Option<String>, // TEST_INSTANCE_ROLE:
    #[structopt(env)]
    test_outputs_path: Option<PathBuf>, // TEST_OUTPUTS_PATH: /outputs
    #[structopt(env)]
    test_run: Option<String>, // TEST_RUN: c7fjstge5te621cen4i0
    #[structopt(env)]
    test_sidecar: Option<bool>, // TEST_SIDECAR: true
    #[structopt(env)]
    test_group_instance_count: Option<usize>, // TEST_GROUP_INSTANCE_COUNT: 1
    #[structopt(env)]
    test_group_id: Option<String>, // TEST_GROUP_ID: single
    #[structopt(env)]
    test_instance_params: Option<String>, // TEST_INSTANCE_PARAMS:
    #[structopt(env)]
    test_repo: Option<String>, //TEST_REPO:
    #[structopt(env)]
    test_tag: Option<String>, // TEST_TAG:
    #[structopt(env)]
    test_disable_metrics: Option<bool>, // TEST_DISABLE_METRICS: false
    #[structopt(env)]
    test_case: Option<String>, // TEST_CASE: quickstart
    #[structopt(env)]
    test_subnet: Option<String>, // TEST_SUBNET: 16.0.0.0/16
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Arguments::from_args();

    let client = Client::open(format!("redis://{}:6379/0", args.redis_host))?;
    let connection = client.get_multiplexed_tokio_connection().await?;

    /* let pong: Option<String> = redis::cmd("PING")
        .query_async(&mut connection.clone())
        .await?;

    if let Some(pong) = pong {
        println!("Sent Ping Received {}", pong);
    } */

    //println!("waiting for network initialization");

    /* barrier(
        connection.clone(),
        "network-initialized",
        args.test_instance_count,
    )
    .await?; */

    //println!("network initilization complete");

    let seq = signal_entry(connection.clone(), "bootstraped").await?;

    println!("I'm intance #{}", seq);

    let seq = barrier(connection.clone(), "bootstraped", args.test_instance_count).await?;

    println!("The count is {}. I'm done... Bye", seq);

    Ok(())
}

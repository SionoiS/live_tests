use std::{borrow::Cow, time::Duration, usize};

use futures_util::stream::Stream;

use tokio::time::sleep;

use redis::{aio::MultiplexedConnection, AsyncCommands, Client, FromRedisValue, Msg, ToRedisArgs};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const INTERVAL: Duration = Duration::from_secs(1);

pub struct SyncClient {
    connection: MultiplexedConnection,
    client: Client,
}

impl SyncClient {
    pub async fn new(redis_host: &str) -> Result<Self> {
        let client = Client::open(format!("redis://{}:6379/0", redis_host))?;
        let connection = client.get_multiplexed_tokio_connection().await?;

        Ok(Self { connection, client })
    }

    pub async fn wait_for_network_initialized(
        &self,
        test_run: String,
        test_plan: String,
        test_case: String,
        test_group_id: String,
        test_instance_count: usize,
    ) -> Result<()> {
        let event_key = format!(
            "run:{}:plan:{}:case:{}:run_events",
            test_run, test_plan, test_case
        );

        let event = format!(
            "{{\"stage_start_event\": {{\"name\": \"network-initialized\", \"group\": \"{}\"}}",
            test_group_id
        );

        println!("Publish\nKey => {}\nEvent => {}", event_key, event);

        self.publish(&event_key, event).await?;

        let state_key = format!(
            "run:{}:plan:{}:case:{}:states:{}",
            test_run, test_plan, test_case, "network-initialized"
        );

        println!("Publish\nKey => {}", state_key);

        let mut barrier = self.barrier(state_key, test_instance_count);
        barrier.down().await?; // Network was initialized

        let event_key = format!(
            "run:{}:plan:{}:case:{}:run_events",
            test_run, test_plan, test_case
        );

        let event = format!(
            "{{\"stage_end_event\": {{\"name\": \"network-initialized\", \"group\": \"{}\"}}",
            test_group_id
        );

        println!("Key => {}\nEvent => {}", event_key, event);

        self.publish(&event_key, event).await?;

        Ok(())
    }

    pub fn barrier(&self, key: impl Into<Cow<'static, str>>, target: usize) -> Barrier {
        let connection = self.connection.clone();
        let key: String = key.into().into_owned();

        Barrier::new(connection, key, target)
    }

    pub async fn publish(
        &self,
        topic: &str,
        message: impl ToRedisArgs + Send + Sync,
    ) -> Result<impl FromRedisValue> {
        let mut connection = self.connection.clone();
        let res = connection.publish(topic, message).await?;

        Ok(res)
    }

    pub async fn signal_entry(&self, key: &str) -> Result<usize> {
        let mut connection = self.connection.clone();
        let seq = connection.incr(key, 1).await?;

        Ok(seq)
    }

    pub async fn subscribe(&self, topic: &str) -> Result<impl Stream<Item = Msg>> {
        let connection = self.client.get_tokio_connection().await?;
        let mut pubsub = connection.into_pubsub();

        pubsub.subscribe(topic).await?;

        Ok(pubsub.into_on_message())
    }
}

pub struct Barrier {
    connection: MultiplexedConnection,
    key: String,
    target: usize,
}

impl Barrier {
    fn new(connection: MultiplexedConnection, key: String, target: usize) -> Self {
        Self {
            connection,
            key,
            target,
        }
    }

    pub async fn down(&mut self) -> Result<()> {
        loop {
            let res: Option<usize> = self.connection.get(&self.key).await?;

            if let Some(res) = res {
                if res >= self.target {
                    return Ok(());
                }
            }

            sleep(INTERVAL).await;
        }
    }
}

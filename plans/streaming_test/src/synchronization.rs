use std::borrow::Cow;
use std::{time::Duration, usize};

use tokio::time::sleep;

use redis::{aio::MultiplexedConnection, AsyncCommands};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const INTERVAL: Duration = Duration::from_secs(1);

pub async fn signal_entry(mut connection: MultiplexedConnection, key: &str) -> Result<usize> {
    let seq = connection.incr(key, 1).await?;

    Ok(seq)
}

pub async fn barrier(
    mut connection: MultiplexedConnection,
    key: impl Into<Cow<'static, str>>,
    target: usize,
) -> Result<usize> {
    let key: String = key.into().into_owned();

    loop {
        let res: Option<usize> = connection.get(&key).await?;

        if let Some(res) = res {
            if res >= target {
                return Ok(res);
            }
        }

        sleep(INTERVAL).await;
    }
}

pub async fn signal_and_wait(
    mut connection: MultiplexedConnection,
    key: impl Into<Cow<'static, str>>,
    target: usize,
) -> Result<usize> {
    let key = key.into().into_owned();

    connection.incr(&key, 1).await?;

    loop {
        let res: Option<usize> = connection.get(&key).await?;

        if let Some(res) = res {
            if res >= target {
                return Ok(res);
            }
        }

        sleep(INTERVAL).await;
    }
}

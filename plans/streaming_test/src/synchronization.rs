use std::borrow::Cow;
use std::{time::Duration, usize};

use tokio::time::sleep;

use redis::{aio::Connection, AsyncCommands, Client};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const INTERVAL: Duration = Duration::from_secs(1);

pub struct Barrier {
    connexion: Connection,
    key: String,
    target: usize,
}

impl Barrier {
    pub async fn new(
        client: &Client,
        key: impl Into<Cow<'static, str>>,
        target: usize,
    ) -> Result<Self> {
        let connexion = client.get_tokio_connection().await?;
        let key = key.into().into_owned();

        Ok(Self {
            connexion,
            key,
            target,
        })
    }

    pub async fn wait(mut self) -> Result<usize> {
        loop {
            let res: usize = match self.connexion.get(&self.key).await {
                Ok(res) => res,
                Err(e) => return Err(e.into()),
            };

            if res >= self.target {
                return Ok(res);
            }

            sleep(INTERVAL).await;
        }
    }
}

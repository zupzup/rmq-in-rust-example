use async_trait::async_trait;
use futures::{join, StreamExt};
use lapin::{options::*, types::FieldTable, BasicProperties, ConnectionProperties};
use std::convert::Infallible;
use std::result::Result as StdResult;
use std::time::Duration;
use thiserror::Error as ThisError;
use tokio_amqp::*;
use warp::{Filter, Rejection, Reply};

type WebResult<T> = StdResult<T, Rejection>;
type RMQResult<T> = StdResult<T, PoolError>;
type Result<T> = StdResult<T, Error>;

type Pool = deadpool::managed::Pool<lapin::Connection, lapin::Error>;
type Connection = deadpool::managed::Object<lapin::Connection, lapin::Error>;

#[derive(ThisError, Debug)]
enum Error {
    #[error("rmq error: {0}")]
    RMQError(#[from] lapin::Error),
    #[error("rmq pool error: {0}")]
    RMQPoolError(#[from] PoolError),
}

impl warp::reject::Reject for Error {}

#[tokio::main]
async fn main() -> Result<()> {
    let addr = std::env::var("AMQP_ADDR")
        .unwrap_or_else(|_| "amqp://rmq:rmq@127.0.0.1:5672/%2f?connection_timeout=3000".into());
    let manager = Manager::new(addr, ConnectionProperties::default().with_tokio());
    let pool = deadpool::managed::Pool::new(manager, 10);

    let health_route = warp::path!("health").and_then(health_handler);
    let add_msg_route = warp::path!("msg")
        .and(warp::post())
        .and(with_rmq(pool.clone()))
        .and_then(add_msg_handler);
    let routes = health_route
        .or(add_msg_route)
        .with(warp::cors().allow_any_origin());

    println!("Started server at localhost:8000");
    let _ = join!(
        warp::serve(routes).run(([0, 0, 0, 0], 8000)),
        rmq_listen(pool.clone())
    );
    Ok(())
}

fn with_rmq(pool: Pool) -> impl Filter<Extract = (Pool,), Error = Infallible> + Clone {
    warp::any().map(move || pool.clone())
}

async fn add_msg_handler(pool: Pool) -> WebResult<impl Reply> {
    let payload = b"Hello world!";

    let rmq_con = get_rmq_con(pool).await.map_err(|e| {
        eprintln!("can't connect to rmq, {}", e);
        warp::reject::custom(Error::RMQPoolError(e))
    })?;

    let channel = rmq_con.create_channel().await.map_err(|e| {
        eprintln!("can't create channel, {}", e);
        warp::reject::custom(Error::RMQError(e))
    })?;

    channel
        .basic_publish(
            "",
            "hello",
            BasicPublishOptions::default(),
            payload.to_vec(),
            BasicProperties::default(),
        )
        .await
        .map_err(|e| {
            eprintln!("can't publish: {}", e);
            warp::reject::custom(Error::RMQError(e))
        })?
        .await
        .map_err(|e| {
            eprintln!("can't publish: {}", e);
            warp::reject::custom(Error::RMQError(e))
        })?;
    Ok("OK")
}

async fn health_handler() -> WebResult<impl Reply> {
    Ok("OK")
}

async fn get_rmq_con(pool: Pool) -> RMQResult<Connection> {
    let connection = pool.get().await?;
    Ok(connection)
}

async fn rmq_listen(pool: Pool) -> Result<()> {
    let mut retry_interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        retry_interval.tick().await;
        println!("connecting rmq consumer...");
        match init_rmq_listen(pool.clone()).await {
            Ok(_) => println!("rmq listen returned"),
            Err(e) => eprintln!("rmq listen had an error: {}", e),
        };
    }
}

async fn init_rmq_listen(pool: Pool) -> Result<()> {
    let rmq_con = get_rmq_con(pool).await.map_err(|e| {
        eprintln!("could not get rmq con: {}", e);
        e
    })?;
    let channel = rmq_con.create_channel().await?;

    let queue = channel
        .queue_declare(
            "hello",
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await?;
    println!("Declared queue {:?}", queue);

    let mut consumer = channel
        .basic_consume(
            "hello",
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    println!("rmq consumer connected, waiting for messages");
    while let Some(delivery) = consumer.next().await {
        if let Ok((channel, delivery)) = delivery {
            println!("received msg: {:?}", delivery);
            channel
                .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
                .await?
        }
    }
    Ok(())
}

// TODO: remove once 1.0.0 deadpool-lapin is merged
pub struct Manager {
    addr: String,
    connection_properties: ConnectionProperties,
}

impl Manager {
    pub fn new(addr: String, connection_properties: ConnectionProperties) -> Self {
        Self {
            addr: addr,
            connection_properties: connection_properties,
        }
    }
}

type PoolError = deadpool::managed::PoolError<lapin::Error>;
type RecycleResult = deadpool::managed::RecycleResult<lapin::Error>;
type RecycleError = deadpool::managed::RecycleError<lapin::Error>;

#[async_trait]
impl deadpool::managed::Manager<lapin::Connection, lapin::Error> for Manager {
    async fn create(&self) -> StdResult<lapin::Connection, lapin::Error> {
        let connection =
            lapin::Connection::connect(self.addr.as_str(), self.connection_properties.clone())
                .await?;
        Ok(connection)
    }

    async fn recycle(&self, connection: &mut lapin::Connection) -> RecycleResult {
        match connection.status().state() {
            lapin::ConnectionState::Connected => Ok(()),
            other_state => Err(RecycleError::Message(format!(
                "lapin connection is in state: {:?}",
                other_state
            ))),
        }
    }
}

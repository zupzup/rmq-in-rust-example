use lapin::{
    message::DeliveryResult, options::*, types::FieldTable, BasicProperties, Connection,
    ConnectionProperties, Result,
};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_amqp::*;
use warp::{Filter, Rejection, Reply};

type WebResult<T> = std::result::Result<T, Rejection>;

#[tokio::main]
async fn main() -> Result<()> {
    let rmq_con = get_rmq_con().await?;
    let shared_rmq_con = Arc::new(rmq_con);
    let channel = shared_rmq_con.create_channel().await?;
    let queue = channel
        .queue_declare(
            "hello",
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await?;

    println!("Declared queue {:?}", queue);

    let consumer = channel
        .basic_consume(
            "hello",
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    consumer.set_delegate(move |delivery: DeliveryResult| async move {
        let delivery = delivery.expect("error caught in in consumer");
        if let Some((channel, delivery)) = delivery {
            println!("received msg: {:?}", delivery);
            channel
                .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
                .await
                .expect("failed to ack");
        }
    })?;

    println!("set up consumer");

    let health_route = warp::path!("health").and_then(health_handler);
    let add_msg_route = warp::path!("msg")
        .and(warp::post())
        .and(with_rmq(shared_rmq_con.clone()))
        .and_then(add_msg_handler);
    let routes = health_route
        .or(add_msg_route)
        .with(warp::cors().allow_any_origin());

    println!("Started server at localhost:8000");
    warp::serve(routes).run(([0, 0, 0, 0], 8000)).await;
    Ok(())
}

fn with_rmq(
    conn: Arc<Connection>,
) -> impl Filter<Extract = (Arc<Connection>,), Error = Infallible> + Clone {
    warp::any().map(move || conn.clone())
}

async fn health_handler() -> WebResult<impl Reply> {
    Ok("OK")
}

async fn add_msg_handler(conn: Arc<Connection>) -> WebResult<impl Reply> {
    let payload = b"Hello world!";
    let channel = conn
        .create_channel()
        .await
        .map_err(|_| warp::reject::reject())?;

    channel
        .basic_publish(
            "",
            "hello",
            BasicPublishOptions::default(),
            payload.to_vec(),
            BasicProperties::default(),
        )
        .await
        .map_err(|_| warp::reject::reject())?
        .await
        .map_err(|_| warp::reject::reject())?;
    Ok("OK")
}

async fn get_rmq_con() -> Result<Connection> {
    let addr = std::env::var("AMQP_ADDR")
        .unwrap_or_else(|_| "amqp://rmq:rmq@127.0.0.1:5672/%2f?connection_timeout=5000".into());
    Connection::connect(&addr, ConnectionProperties::default().with_tokio()).await
}

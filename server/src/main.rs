use std::env::args;

use axum::{
    routing::get,
    Router,
};
use anyhow::Error;
use rstar;
use rustmatica::BlockState;

mod names;


struct ControlState {
    //turtles: Vec<Turtle>,
    //world: unimplemented!(),
    //chunkloaders: unimplemented!(),


}

#[tokio::main]
async fn main() -> Result<(), Error> {
    println!("{}", names::Name::from_num(args().nth(1).unwrap().parse().unwrap()).to_str());
    println!("{:?}", feistel_rs::feistel_encrypt(&[127,127], &[127,127], 1));
    let serv = Router::new().route("/", get(|| async { "Hello" }));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:48228").await.unwrap();

    axum::serve(listener, serv).await.unwrap();

    Ok(())
}

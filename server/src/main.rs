use std::{env::args, sync::Arc, fs, io::ErrorKind};

use axum::{
    routing::{get, post},
    Router, extract::{State, Path}, Json, http::{request, Request},
};
use anyhow::{Error, Ok};
use rstar;
use rustmatica::{BlockState, util::{UVec3, Vec3}};

mod names;
use names::Name;
use serde_json::Value;
use tokio::{sync::{Mutex, RwLock, watch}, signal};
use serde::{Serialize, Deserialize};
use const_format::formatcp;
use hyper_util::rt::TokioIo;
use tower::Service;
use hyper::body::Incoming;

#[derive(Serialize, Deserialize)]
enum Direction {
    North,
    South,
    East,
    West,
}

impl Direction {
    fn left(self) -> Self {
        match self {
            Direction::North => Direction::West,
            Direction::South => Direction::East,
            Direction::East => Direction::North,
            Direction::West => Direction::South,
        }
    }
    
    fn right(self) -> Self {
        match self {
            Direction::North => Direction::East,
            Direction::South => Direction::West,
            Direction::East => Direction::South,
            Direction::West => Direction::North,
        }
    }
    fn unit(self) -> Vec3 {
        match self {
            Direction::North => Vec3::new(0, 0, -1),
            Direction::South => Vec3::new(0, 0, 1),
            Direction::East => Vec3::new(1, 0, 0),
            Direction::West => Vec3::new(-1, 0, 0),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Turtle {
    name: Name,
    fuel: usize,
    /// movement vector of last given command
    queued_movement: Vec3,
    position: Vec3,
    facing: Direction,
}

impl Turtle {
    fn new(id: u32, position: Vec3, facing: Direction, fuel: usize) -> Self {
        Self { name: Name::from_num(id), fuel, queued_movement: Vec3::new(0, 0, 0), position, facing }

    }
}

#[derive(Serialize, Deserialize)]
struct ControlState {
    turtles: Vec<Turtle>
    //world: unimplemented!(),
    //chunkloaders: unimplemented!(),
}

type SharedControl = Arc<RwLock<ControlState>>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    
    let state = match tokio::fs::OpenOptions::new().read(true).open("state.json").await {
        tokio::io::Result::Ok(file) => {
            serde_json::from_reader(file.into_std().await)?
        },
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => {
                ControlState { turtles: Vec::new() }
            },
            _ => panic!()
        }
    };

    let state = SharedControl::new(RwLock::new(state));

    let serv = Router::new()
        .route("/turtle/new", post(create_turtle))
        .route("/turtle/update/:id", post(command))
        .route("/turtle/client.lua", get(client))
    .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:48228").await.unwrap();

    let (close_tx, close_rx) = watch::channel(());

    loop {
        let (socket, remote_addr) = tokio::select! {
            result = listener.accept() => {
                result.unwrap()
            }
            _ = shutdown_signal() => {
                println!("cancelled connection");
                break;
            }
        };

        let tower = serv.clone();
        let close_rx = close_rx.clone();

        tokio::spawn(async move {
            let socket = TokioIo::new(socket);
            let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
                tower.clone().call(request)
            });

            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(socket, hyper_service)
                .with_upgrades(); // future

            let mut conn = std::pin::pin!(conn);

            loop {
                tokio::select! {
                    result = conn.as_mut() => {
                        if result.is_err() {
                            println!("req failed");
                        }
                        break;
                    }
                    _ = shutdown_signal() => {
                        println!("starting shutdown");
                        conn.as_mut().graceful_shutdown();
                    }
                }
            }

            drop(close_rx);
        });
    };

    write_to_disk(state).await?;

    Ok(())
}

async fn write_to_disk(state: SharedControl) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(&(*state.read().await))?;
    tokio::fs::write("state.json", json).await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await.unwrap();
    };

    ctrl_c.await
}

async fn create_turtle(
    State(state): State<SharedControl>,
    Json(req): Json<TurtleRegister>,
    ) -> Json<TurtleResponse> {
    let turtles = &mut state.write().await.turtles;
    let id = (turtles.len() + 1) as u32;
    turtles.push(Turtle::new(id, req.position, req.facing, req.fuel));

    println!("turt {id}");

    Json(TurtleResponse {name: Name::from_num(id).to_str(), id, command: TurtleCommand::Update})
}

async fn command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<TurtleUpdate>,
    ) -> Json<TurtleCommand> {
    let turtles = &state.read().await.turtles;
    println!("{id}");
    println!("above: {}, below: {}, ahead: {}", req.above, req.below, req.ahead);


    Json(TurtleCommand::Wait)
}

#[derive(Serialize, Deserialize)]
enum TurtleCommand {
    Wait,
    Forward,
    Backward,
    Up,
    Down,
    Left,
    Right,
    Dig,
    DigUp,
    DigDown,
    TakeInventory,
    Update,
    Poweroff,
}

#[derive(Serialize, Deserialize)]
struct TurtleUpdate {
    fuel: usize,
    /// Block name
    ahead: String,
    above: String,
    below: String,
}

#[derive(Serialize, Deserialize)]
struct TurtleRegister {
    fuel: usize,
    position: Vec3,
    facing: Direction,
}

#[derive(Serialize, Deserialize)]
struct TurtleResponse {
    name: String,
    id: u32,
    command: TurtleCommand,
}

async fn client() -> &'static str {
    formatcp!("local ipaddr = {}\n{}", include_str!("../ipaddr.txt"), include_str!("../../client/client.lua"))
}

#![feature(iter_map_windows, iter_collect_into)]

use std::{collections::VecDeque, io::ErrorKind, sync::Arc, env::args, path};

use anyhow::{Context, Error, Ok};
use axum::{
    extract::{Path, State},
    http::request,
    routing::{get, post},
    Json, Router,
};
use blocks::{World, Position, Vec3};
use indoc::formatdoc;
use mine::TurtleMineJob;
use rstar::{self, AABB, RTree};

use const_format::formatcp;
use hyper::body::Incoming;
use nalgebra::Vector3;
use names::Name;
use serde::{Deserialize, Serialize};
use tokio::{sync::{
    watch::{self},
    Mutex, RwLock, mpsc, OnceCell
}, fs};
use tower::Service;
use turtle::{TurtleTask, Iota, Receiver, Sender, Turtle, TurtleUpdate, TurtleInfo, TurtleCommand, TurtleCommander, TurtleCommandResponse};

use crate::{blocks::Block, paths::route};

mod blocks;
mod names;
mod mine;
mod paths;
mod safe_kill;
mod turtle;

#[derive(Serialize, Deserialize)]
struct SavedState {
    turtles: Vec<turtle::Turtle>,
    world: RTree<Block>,
    //chunkloaders: unimplemented!(),
}

struct LiveState {
    turtles: Vec<Arc<RwLock<turtle::Turtle>>>,
    tasks: Vec<VecDeque<TurtleMineJob>>,
    world: blocks::World,
}

impl LiveState {
    async fn save(&self) -> SavedState {
        let mut turtles = Vec::new();
        for turtle in self.turtles.iter() {
            turtles.push(turtle.read().await.info());
        };
        SavedState { turtles, world: self.world.tree().await }
    }

    fn from_save(save: SavedState) -> Self {
        let mut turtles = Vec::new();
        for turtle in save.turtles.into_iter() {
            let (tx, rx) = mpsc::channel(1);
            turtles.push(Turtle::with_channel(turtle.name.to_num(), turtle.position, turtle.fuel, turtle.fuel_limit, tx, rx));
        };
        Self { turtles: turtles.into_iter().map(|t| Arc::new(RwLock::new(t))).collect(), tasks: Vec::new(), world: World::from_tree(save.world) }
    }

    async fn get_turtle(&self, name: u32) -> Option<TurtleCommander> {
        TurtleCommander::new(Name::from_num(name), self).await
    }
    
}

static PORT: OnceCell<u16> = OnceCell::const_new();
static SAVE: OnceCell<path::PathBuf> = OnceCell::const_new();

type SharedControl = Arc<RwLock<LiveState>>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut args = args().skip(1);
    PORT.set(match args.next() {
        Some(port) => port.parse()?,
        None => 48228,
    })?;
    SAVE.set(match args.next() {
        Some(file) => file.into(),
        None => "save".into(),
    })?;

    let state = read_from_disk().await?;

    let state = LiveState::from_save(state);

    let state = SharedControl::new(RwLock::new(state));

    let server = Router::new()
        .route("/turtle/new", post(create_turtle))
        .route("/turtle/:id/update", post(command))
        .route("/turtle/client.lua", get(client))
        .route("/turtle/:id/setGoal", post(set_goal))
        .route("/turtle/:id/dig", post(dig))
        .route("/turtle/:id/cancelTask", post(cancel))
        .route("/turtle/:id/manual", post(run_command))
        .route("/turtle/:id/info", get(turtle_info))
        //.route("/turtle/:id/placeUp", get(place_up))
        .route("/turtle/updateAll", get(update_turtles))
        .route("/flush", get(flush))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", *PORT.get().unwrap()))
        .await.unwrap();

    let server = safe_kill::serve(server, listener).await;

    println!("writing");
    write_to_disk(state.read().await.save().await).await?;

    server.closed().await;

    Ok(())
}

async fn flush(State(state): State<SharedControl>) -> &'static str {
    write_to_disk(state.read().await.save().await).await.unwrap();

    "ACK"
}

async fn write_to_disk(state: SavedState) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(&state.turtles)?;
    let bincode = bincode::serialize(&state.world)?;
    tokio::fs::write(SAVE.get().unwrap().join("turtles.json"), json).await?;
    tokio::fs::write(SAVE.get().unwrap().join("world.bin"), bincode).await?;
    Ok(())
}

async fn read_from_disk() -> anyhow::Result<SavedState> {
    let turtles = match tokio::fs::OpenOptions::new()
        .read(true)
        .open(SAVE.get().unwrap().join("turtles.json"))
        .await
    {
        tokio::io::Result::Ok(file) => serde_json::from_reader(file.into_std().await)?,
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => Vec::new(),
            _ => panic!(),
        },
    };

    let world = match tokio::fs::OpenOptions::new()
    .read(true).open(SAVE.get().unwrap().join("world.bin")).await {
        tokio::io::Result::Ok(file) => bincode::deserialize_from(file.into_std().await)?,
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => RTree::new(),
            _ => panic!(),
        },
        
    };

    Ok(SavedState {
        turtles,
        world,
    })
}

async fn create_turtle(
    State(state): State<SharedControl>,
    Json(req): Json<turtle::TurtleRegister>,
) -> Json<turtle::TurtleResponse> {
    let state = &mut state.write().await;
    let id = state.turtles.len() as u32;
    let (send, receive) = mpsc::channel(1);
    state.turtles.push(
        Arc::new(RwLock::new(
            turtle::Turtle::with_channel(id, Position::new(req.position, req.facing), req.fuel, req.fuellimit, send,receive)
    )));
    state.tasks.push(VecDeque::new());
    

    println!("new turtle: {id}");

    Json(turtle::TurtleResponse {
        name: Name::from_num(id).to_str(),
        id,
        command: turtle::TurtleCommand::Update,
    })
}

async fn place_up(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> Json<TurtleInfo> {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    let response = turtle.execute(turtle::TurtleCommand::PlaceUp).await;

    Json(response)
}

async fn run_command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<TurtleCommand>,
) -> Json<TurtleCommandResponse> {
    let state = state.read().await;
    let commander = state.get_turtle(id).await.unwrap().clone();
    drop(state);
    Json(commander.execute(req).await.ret)
}

async fn dig(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Vec3>,
) -> &'static str {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    let fuel = Position::new(Vec3::new(-19, 93, 73), blocks::Direction::East);
    let inventory = Position::new(Vec3::new(-19, 92, 73), blocks::Direction::East);
    tokio::spawn(
        async move {
            mine::mine(turtle.clone(), req, fuel, inventory).await
        }
    );

    "ACK"
}

async fn set_goal(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Position>,
) -> &'static str {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    tokio::spawn(async move {turtle.goto(req).await});

    "ACK"
}

async fn cancel(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> &'static str {
    state.write().await.tasks[id as usize].pop_front();

    "ACK"
}

async fn update_turtles(State(state): State<SharedControl>) -> &'static str {
    for turtle in state.read().await.turtles.iter() {
            turtle.write().await.pending_update = true;
    }

    "ACK"
}

async fn turtle_info(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> Json<turtle::Turtle> {
    let state = &mut state.read().await;
    let turtle = &state.turtles[id as usize].read().await;

    let cloned = Turtle::new( 
        turtle.name.to_num(),
        turtle.position,
        turtle.fuel
    );

    Json(cloned)
}

async fn command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<turtle::TurtleUpdate>,
) -> Json<turtle::TurtleCommand> {
    let mut state = &mut state.write().await;

    if id as usize > state.turtles.len() {
        return Json(turtle::TurtleCommand::Update);
    }

    Json(
        turtle::process_turtle_update(id, &mut state, req).await
        .unwrap_or(turtle::TurtleCommand::Update)
    )
}

async fn client() -> String {
    formatdoc!(r#"
        local ipaddr = {}
        local port = "{}"
        {}"#,
        include_str!("../ipaddr.txt"),
        PORT.get().unwrap(),
        fs::read_to_string("../client/client.lua").await.unwrap(), // TODO: cache handle if bottleneck
    )
}


#![feature(iter_map_windows)]

use std::{collections::VecDeque, io::ErrorKind, sync::Arc};

use anyhow::{Context, Error, Ok};
use axum::{
    extract::{Path, State},
    http::request,
    routing::{get, post},
    Json, Router,
};
use blocks::{World, Position, Vec3};
use mine::TurtleMineJob;
use rstar::{self, AABB, RTree};

use const_format::formatcp;
use hyper::body::Incoming;
use nalgebra::Vector3;
use names::Name;
use serde::{Deserialize, Serialize};
use tokio::sync::{
    watch::{self},
    Mutex, RwLock, mpsc
};
use tower::Service;
use turtle::{TurtleTask, Iota, Receiver, Sender, Turtle, TurtleUpdate, TurtleInfo, TurtleCommand, TurtleCommander};

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


type SharedControl = Arc<RwLock<LiveState>>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let state = match tokio::fs::OpenOptions::new()
        .read(true)
        .open("state.json")
        .await
    {
        tokio::io::Result::Ok(file) => serde_json::from_reader(file.into_std().await)?,
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => SavedState {
                turtles: Vec::new(),
                world: RTree::new(),
            },
            _ => panic!(),
        },
    };

    let state = LiveState::from_save(state);

    let state = SharedControl::new(RwLock::new(state));

    let server = Router::new()
        .route("/turtle/new", post(create_turtle))
        .route("/turtle/:id/update", post(command))
        .route("/turtle/client.lua", get(client))
        .route("/turtle/:id/setGoal", post(set_goal))
        .route("/turtle/:id/dig", post(dig))
        .route("/turtle/:id/cancelTask", post(cancel))
        .route("/turtle/:id/info", get(turtle_info))
        //.route("/turtle/:id/placeUp", get(place_up))
        .route("/turtle/updateAll", get(update_turtles))
        .route("/flush", get(flush))
        .with_state(state.clone());

    let server = safe_kill::serve(server).await;

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
    let json = serde_json::to_string_pretty(&state)?;
    tokio::fs::write("state.json", json).await?;
    Ok(())
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

async fn dig(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Vec3>,
) -> &'static str {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    tokio::spawn(
        async move {
            mine::mine_chunk(turtle.clone(), req).await
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

async fn client() -> &'static str {
    formatcp!(
        "local ipaddr = {}\n{}",
        include_str!("../ipaddr.txt"),
        include_str!("../../client/client.lua")
    )
}


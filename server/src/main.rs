use std::{collections::VecDeque, io::ErrorKind, sync::Arc};

use anyhow::{Context, Error, Ok};
use axum::{
    extract::{Path, State},
    http::request,
    routing::{get, post},
    Json, Router,
};
use blocks::{World, Position};
use mine::TurtleMineJob;
use rstar::{self, AABB};

use const_format::formatcp;
use hyper::body::Incoming;
use nalgebra::Vector3;
use names::Name;
use serde::{Deserialize, Serialize};
use tokio::sync::{
    watch::{self},
    Mutex, RwLock,
};
use tower::Service;
use turtle::TurtleTask;

use crate::{blocks::Block, paths::route};

mod blocks;
mod names;
mod mine;
mod paths;
mod safe_kill;
mod turtle;

#[derive(Serialize, Deserialize)]
struct ControlState {
    turtles: Vec<turtle::Turtle>,
    tasks: Vec<VecDeque<TurtleMineJob>>,
    world: blocks::World,
    //chunkloaders: unimplemented!(),
}

type SharedControl = Arc<RwLock<ControlState>>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let state = match tokio::fs::OpenOptions::new()
        .read(true)
        .open("state.json")
        .await
    {
        tokio::io::Result::Ok(file) => serde_json::from_reader(file.into_std().await)?,
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => ControlState {
                turtles: Vec::new(),
                world: World::new(),
                tasks: Vec::new(),
            },
            _ => panic!(),
        },
    };

    let state = SharedControl::new(RwLock::new(state));

    let server = Router::new()
        .route("/turtle/new", post(create_turtle))
        .route("/turtle/:id/update", post(command))
        .route("/turtle/client.lua", get(client))
        .route("/turtle/:id/setGoal", post(set_goal))
        .route("/turtle/:id/info", get(turtle_info))
        .route("/turtle/updateAll", get(update_turtles))
        .route("/flush", get(flush))
        .with_state(state.clone());

    let server = safe_kill::serve(server).await;

    println!("writing");
    write_to_disk(state).await?;

    server.closed().await;

    Ok(())
}


async fn write_to_disk(state: SharedControl) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(&(*state.read().await))?;
    tokio::fs::write("state.json", json).await?;
    Ok(())
}

async fn flush(State(state): State<SharedControl>) -> &'static str {
    write_to_disk(state).await.unwrap();

    "ACK"
}

async fn create_turtle(
    State(state): State<SharedControl>,
    Json(req): Json<turtle::TurtleRegister>,
) -> Json<turtle::TurtleResponse> {
    let state = &mut state.write().await;
    let turtles = &mut state.turtles;
    let id = turtles.len() as u32;
    turtles.push(turtle::Turtle::new(id, req.position, req.facing, req.fuel));
    state.tasks.push(VecDeque::new());
    

    println!("turt {id}");

    Json(turtle::TurtleResponse {
        name: Name::from_num(id).to_str(),
        id,
        command: turtle::TurtleCommand::Update,
    })
}

async fn set_goal(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Position>,
) -> &'static str {
    state.write().await.tasks[id as usize].push_back(
        TurtleMineJob::chunk(req.0)
    );

    "ACK"
}

async fn update_turtles(State(state): State<SharedControl>) -> &'static str {
    state
        .write()
        .await
        .turtles
        .iter_mut()
        .for_each(|t| t.pending_update = true);
    "ACK"
}

async fn turtle_info(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> Json<turtle::Turtle> {
    let state = &mut state.read().await;
    let turtle = &state.turtles[id as usize];

    let cloned = turtle::Turtle {
        name: turtle.name.clone(),
        fuel: turtle.fuel,
        queued_movement: turtle.queued_movement.clone(),
        position: turtle.position.clone(),
        goal: turtle.goal.clone(),
        pending_update: turtle.pending_update,
    };

    Json(cloned)
}

async fn command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<turtle::TurtleUpdate>,
) -> Json<turtle::TurtleCommand> {
    let mut state = &mut state.write().await;

    println!("{:?}", &req);

    if id as usize > state.turtles.len() {
        return Json(turtle::TurtleCommand::Update);
    }

    Json(
        turtle::process_turtle_update(id, &mut state, req).unwrap_or(turtle::TurtleCommand::Update),
    )
}

async fn client() -> &'static str {
    formatcp!(
        "local ipaddr = {}\n{}",
        include_str!("../ipaddr.txt"),
        include_str!("../../client/client.lua")
    )
}


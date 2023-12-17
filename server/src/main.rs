use std::{env::args, sync::Arc, fs, io::ErrorKind, collections::VecDeque};

use axum::{
    routing::{get, post},
    Router, extract::{State, Path}, Json, http::{request},
};
use anyhow::{Error, Ok, Context};
use blocks::World;
use rstar::{self, AABB};

mod names;
use names::Name;
use tokio::{sync::{Mutex, RwLock, watch::{self}}};
use serde::{Serialize, Deserialize};
use const_format::formatcp;
use hyper_util::rt::TokioIo;
use tower::Service;
use hyper::body::Incoming;
use nalgebra::Vector3;

use crate::{blocks::Block, paths::route};
mod blocks;

mod paths;

pub type Vec3 = Vector3<i32>;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy, Debug)]
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
struct ControlState {
    turtles: Vec<turtle::Turtle>,
    world: blocks::World,
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
                ControlState { turtles:Vec::new(), world: World::new() }
            },
            _ => panic!()
        }
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

mod safe_kill; 

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
    let turtles = &mut state.write().await.turtles;
    let id = turtles.len() as u32;
    turtles.push(turtle::Turtle::new(id, req.position, req.facing, req.fuel));

    println!("turt {id}");

    Json(turtle::TurtleResponse {name: Name::from_num(id).to_str(), id, command: turtle::TurtleCommand::Update})
}

async fn set_goal(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Position>,
    ) -> &'static str {
    state.write().await.turtles[id as usize].goal = Some(req);

    "ACK"
}

async fn update_turtles(
    State(state): State<SharedControl>,
    ) -> &'static str {
    state.write().await.turtles.iter_mut().for_each(|t| t.pending_update = true);
    "ACK"
}

async fn turtle_info(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    ) -> Json<turtle::Turtle> {
    let state = &mut state.read().await;
    let turtle = &state.turtles[id as usize];

    let mut pseudomoves: VecDeque<turtle::TurtleCommand> = VecDeque::new();
    turtle.moves.front().map(|m| pseudomoves.push_front(m.clone()));

    let cloned = turtle::Turtle {
        name: turtle.name.clone(),
        fuel: turtle.fuel,
        queued_movement: turtle.queued_movement.clone(),
        position: turtle.position.clone(),
        goal: turtle.goal.clone(),
        pending_update: turtle.pending_update,
        moves: pseudomoves,
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

type Position = (Vec3, Direction);

/// Get a turtle command to map two adjacent positions
fn difference(from: Position, to: Position) -> Option<turtle::TurtleCommand> {
    use turtle::TurtleCommand::*;

    if from.0 == to.0 {
        if to.1 == from.1.left() {
            Some(Left)
        } else if to.1 == from.1.right() {
            Some(Right)
        } else {
            None
        }
    } else if to.1 == from.1 {
        if to.0 == from.0 + from.1.unit() {
            Some(Forward(1))
        } else if to.0 == from.0 - from.1.unit() {
            Some(Backward(1))
        } else if to.0 == from.0 + Vec3::y() {
            Some(Up(1))
        } else if to.0 == from.0 - Vec3::y() {
            Some(Down(1))
        } else {
            None
        }
        
    } else {
        None
    }
}

#[derive(Serialize, Deserialize)]
struct TurtleMineJobParams {
    region: AABB<[i32;3]>,
    to_mine: Vec<Vec3>,
    method: TurtleMineMethod,
    refuel: Position,
    storage: Position,
}

#[derive(Serialize, Deserialize)]
struct TurtleMineJob {
    to_mine: VecDeque<Vec3>,
    mined: AABB<[i32;3]>,
    params: TurtleMineJobParams,
}


#[derive(Serialize, Deserialize)]
enum TurtleMineMethod {
    Clear,
    Strip,
}
async fn client() -> &'static str {
    formatcp!("local ipaddr = {}\n{}", include_str!("../ipaddr.txt"), include_str!("../../client/client.lua"))
}

mod turtle;

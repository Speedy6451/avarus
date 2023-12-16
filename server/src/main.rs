use std::{env::args, sync::Arc, fs, io::ErrorKind};

use axum::{
    routing::{get, post},
    Router, extract::{State, Path}, Json,
};
use anyhow::Error;
use rstar;
use rustmatica::{BlockState, util::{UVec3, Vec3}};

mod names;
use names::Name;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use serde::{Serialize, Deserialize};

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
    println!("{}", names::Name::from_num(args().nth(1).unwrap().parse().unwrap()).to_str());

    let state = match fs::File::open("state.json") {
        Ok(file) => {
            serde_json::from_reader(file)?
        },
        Err(e) => match e.kind() {
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
    .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:48228").await.unwrap();

    axum::serve(listener, serv).await.unwrap();

    Ok(())
}

async fn create_turtle(
    State(state): State<SharedControl>,
    Json(req): Json<TurtleRegister>,
    ) -> Json<TurtleResponse> {
    let turtles = &mut state.write().await.turtles;
    let id = (turtles.len() + 1) as u32;
    turtles.push(Turtle::new(id, req.position, req.facing, req.fuel));

    Json(TurtleResponse {name: Name::from_num(id).to_str(), command: TurtleCommand::Wait})
}

async fn command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<TurtleUpdate>,
    ) -> Json<TurtleCommand> {
    let turtles = &state.read().await.turtles;
    println!("{id}");


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
}

#[derive(Serialize, Deserialize)]
struct TurtleUpdate {
    fuel: usize,
    /// Block name
    ahead: String,
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
    command: TurtleCommand,
}

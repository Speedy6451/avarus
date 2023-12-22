use tokio;
use blocks::Vec3;
use crate::turtle::TurtleCommandResponse;
use crate::turtle::TurtleCommander;
use crate::turtle::TurtleInfo;
use axum::extract::Path;
use crate::turtle::TurtleCommand;
use crate::names::Name;
use log::info;
use std::collections::VecDeque;
use blocks::Position;
use crate::turtle::Turtle;
use tokio::sync::RwLock;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::turtle;
use axum::Json;
use axum::extract::State;
use axum::routing::get;
use axum::routing::post;
use crate::blocks;
use crate::mine;
use super::SharedControl;
use axum::Router;
use indoc::formatdoc;
use crate::PORT;
use tokio::fs;

pub fn turtle_api() -> Router<SharedControl> {
    Router::new()
        .route("/new", post(create_turtle))
        .route("/:id/update", post(command))
        .route("/client.lua", get(client))
        .route("/:id/setGoal", post(set_goal))
        .route("/:id/dig", post(dig))
        .route("/:id/cancelTask", post(cancel))
        .route("/:id/manual", post(run_command))
        .route("/:id/info", get(turtle_info))
        .route("/updateAll", get(update_turtles))
}

pub(crate) async fn create_turtle(
    State(state): State<SharedControl>,
    Json(req): Json<turtle::TurtleRegister>,
) -> Json<turtle::TurtleResponse> {
    let state = &mut state.write().await;
    let id = state.turtles.len() as u32;
    let (send, receive) = mpsc::channel(1);
    let turtle = turtle::Turtle::with_channel(id, Position::new(req.position, req.facing), req.fuel, req.fuellimit, send,receive);
    let commander = TurtleCommander::with_turtle(&turtle, state);
    state.tasks.add_turtle(&commander);
    state.turtles.push(
        Arc::new(RwLock::new(
            turtle
    )));

    info!("new turtle: {id}");

    Json(turtle::TurtleResponse {
        name: Name::from_num(id).to_str(),
        id,
        command: turtle::TurtleCommand::Update,
    })
}

pub(crate) async fn place_up(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> Json<TurtleInfo> {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    let response = turtle.execute(turtle::TurtleCommand::PlaceUp).await;

    Json(response)
}

pub(crate) async fn run_command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<TurtleCommand>,
) -> Json<TurtleCommandResponse> {
    let state = state.read().await;
    let commander = state.get_turtle(id).await.unwrap().clone();
    drop(state);
    Json(commander.execute(req).await.ret)
}

pub(crate) async fn dig(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<(Vec3, Position, Position)>,
) -> &'static str {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    let (req, fuel, inventory) = req;
    //let fuel = Position::new(Vec3::new(-19, 93, 73), blocks::Direction::East);
    //let inventory = Position::new(Vec3::new(-19, 92, 73), blocks::Direction::East);
    tokio::spawn(
        async move {
            mine::mine(turtle.clone(), req, fuel, inventory).await
        }
    );

    "ACK"
}

pub(crate) async fn set_goal(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Position>,
) -> &'static str {
    let turtle = state.read().await.get_turtle(id).await.unwrap();
    tokio::spawn(async move {turtle.goto(req).await.expect("route failed")});

    "ACK"
}

pub(crate) async fn cancel(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> &'static str {
    //state.write().await.tasks

    "ACK"
}

pub(crate) async fn update_turtles(State(state): State<SharedControl>) -> &'static str {
    for turtle in state.read().await.turtles.iter() {
            turtle.write().await.pending_update = true;
    }

    "ACK"
}

pub(crate) async fn turtle_info(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> Json<turtle::Turtle> {
    let state = &mut state.read().await;
    let turtle = &state.turtles[id as usize].read().await;

    let cloned = Turtle::new( 
        turtle.name.to_num(),
        turtle.position,
        turtle.fuel,
        turtle.fuel_limit,
    );

    Json(cloned)
}

pub(crate) async fn command(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<turtle::TurtleUpdate>,
) -> Json<turtle::TurtleCommand> {
    let mut state = &mut state.read().await;

    if id as usize > state.turtles.len() {
        return Json(turtle::TurtleCommand::Update);
    }

    Json(
        turtle::process_turtle_update(id, &mut state, req).await
        .unwrap_or(turtle::TurtleCommand::Update)
    )
}

pub(crate) async fn client() -> String {
    formatdoc!(r#"
    local ipaddr = {}
    local port = "{}"
    {}"#,
        include_str!("../ipaddr.txt"),
        PORT.get().unwrap(),
        fs::read_to_string("../client/client.lua").await.unwrap(), // TODO: cache handle if bottleneck
    )
}

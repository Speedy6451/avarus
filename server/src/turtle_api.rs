use log::trace;
use tokio;
use blocks::Vec3;
use tokio::time::Instant;
use crate::fell::TreeFarm;
use crate::mine::Mine;
use crate::mine::Quarry;
use crate::turtle::IDLE_TIME;
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

/// Time (s) after boot to start allocating turtles to tasks
/// too short of a time could make fast-booting turtles do far away tasks over closer ones
const STARTUP_ALLOWANCE: f64 = 4.0;

pub fn turtle_api() -> Router<SharedControl> {
    Router::new()
        .route("/new", post(create_turtle))
        .route("/:id/update", post(command))
        .route("/client.lua", get(client))
        .route("/:id/setGoal", post(set_goal))
        .route("/:id/cancelTask", post(cancel))
        .route("/:id/manual", post(run_command))
        .route("/:id/dock", post(dock))
        .route("/:id/info", get(turtle_info))
        .route("/createTreeFarm", post(fell))
        .route("/createMine", post(dig))
        .route("/registerDepot", post(new_depot))
        .route("/pollScheduler", get(poll))
        .route("/shutdown", get(shutdown)) // probably tramples the rfc
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

pub(crate) async fn dock(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> Json<usize> {
    let state = state.read().await;
    let commander = state.get_turtle(id).await.unwrap().clone();
    drop(state);
    Json(commander.dock().await)
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
    State(state): State<SharedControl>,
    Json(req): Json<Vec3>,
) -> &'static str {
    let schedule = &mut state.write().await.tasks;
    let size = Vec3::new(16,16,16);
    schedule.add_task(Box::new(Quarry::new(req,req+size)));

    "ACK"
}

pub(crate) async fn new_depot(
    State(state): State<SharedControl>,
    Json(req): Json<Position>,
) -> &'static str {
    let depots = &state.read().await.depots;
    depots.add(req).await;

    "ACK"
}

pub(crate) async fn poll(
    State(state): State<SharedControl>,
) -> &'static str {
    let schedule = &mut state.write().await.tasks;
    schedule.poll().await;

    "ACK"
}

pub(crate) async fn shutdown(
    State(state): State<SharedControl>,
) -> &'static str {
    let signal = {
        let mut state = state.write().await;
        let signal = state.tasks.shutdown();
        state.tasks.poll().await;
        signal
    };

    info!("waiting for tasks to finish");
    signal.await.unwrap();

    let state = state.write().await;
    info!("waiting for connections to finish");
    state.kill.send(()).unwrap();

    "ACK"
}

pub(crate) async fn fell(
    State(state): State<SharedControl>,
    Json(req): Json<Vec3>,
) -> &'static str {
    let schedule = &mut state.write().await.tasks;
    schedule.add_task(Box::new(TreeFarm::new(req)));

    "ACK"
}

pub(crate) async fn set_goal(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
    Json(req): Json<Position>,
) -> &'static str {
    let turtle = state.read().await.get_turtle(id).await.unwrap().clone();
    drop(state);
    tokio::spawn(async move {turtle.goto(req).await.expect("route failed")});

    "ACK"
}

pub(crate) async fn cancel(
    Path(id): Path<u32>,
    State(state): State<SharedControl>,
) -> &'static str {
    state.write().await.tasks.cancel(Name::from_num(id)).await;

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
    trace!("reply from turtle {id}: {req:?}");
    let state_guard = state.clone().read_owned().await;

    if id as usize > state_guard.turtles.len() {
        return Json(turtle::TurtleCommand::Update);
    }

    let command = turtle::process_turtle_update(id, &state_guard, req).await;

    let command = match command {
        Some(command) => command,
        None => {
            tokio::spawn(async move {
                let state = &state.clone();
                if Instant::elapsed(&state.clone().read().await.started).as_secs_f64() > STARTUP_ALLOWANCE {
                    let schedule = &mut state.write().await.tasks;
                    trace!("idle, polling");
                    schedule.poll().await;
                }
            });
            turtle::TurtleCommand::Wait(IDLE_TIME)
        },
    };

    Json(command)
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

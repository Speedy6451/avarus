#![feature(iter_map_windows, iter_collect_into)]

use std::{collections::VecDeque, io::ErrorKind, sync::Arc, env::args, path};

use anyhow::{Error, Ok};
use axum::{
    extract::{State},
    routing::{get},
    Router,
};
use blocks::{World, Position, };
use depot::Depots;
use log::info;
use rstar::RTree;

use names::Name;
use tasks::Scheduler;
use tokio::{sync::{
    RwLock, mpsc, OnceCell, Mutex
}, fs, time::Instant};
use turtle::{Turtle, TurtleCommander};
use serde::{Deserialize, Serialize};
use indoc::formatdoc;

use crate::blocks::Block;

mod blocks;
mod names;
mod mine;
mod fell;
mod paths;
mod safe_kill;
mod turtle;
mod turtle_api;
mod tasks;
mod depot;

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

    log4rs::init_file(SAVE.get().unwrap().join("log.yml"), Default::default())?;

    let state = read_from_disk().await?;

    let state = SharedControl::new(RwLock::new(state));

    let server = Router::new()
        //.route("/turtle/:id/placeUp", get(place_up))
        .route("/flush", get(flush))
        .nest("/turtle", turtle_api::turtle_api())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", *PORT.get().unwrap()))
        .await.unwrap();

    let server = safe_kill::serve(server, listener).await;

    info!("writing");
    write_to_disk(&*state.read().await).await?;
    info!("written");

    server.closed().await;

    Ok(())
}

async fn flush(State(state): State<SharedControl>) -> &'static str {
    write_to_disk(&*state.read().await).await.unwrap();

    "ACK"
}

async fn write_to_disk(state: &LiveState) -> anyhow::Result<()> {
    let tasks = &state.tasks;
    let state = state.save().await;

    let turtles = serde_json::to_string_pretty(&state.turtles)?;
    let world = bincode::serialize(&state.world)?;
    let depots = serde_json::to_string_pretty(&state.depots)?;
    let tasks = serde_json::to_string_pretty(tasks)?;

    let path = &SAVE.get().unwrap();
    tokio::fs::write(path.join("turtles.json"), turtles).await?;
    tokio::fs::write(path.join("depots.json"), depots).await?;
    tokio::fs::write(path.join("tasks.json"), tasks).await?;
    tokio::fs::write(path.join("world.bin"), world).await?;
    Ok(())
}

async fn read_from_disk() -> anyhow::Result<LiveState> {
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

    let depots = match tokio::fs::OpenOptions::new()
        .read(true)
        .open(SAVE.get().unwrap().join("depots.json"))
        .await
    {
        tokio::io::Result::Ok(file) => serde_json::from_reader(file.into_std().await)?,
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => Vec::new(),
            _ => panic!(),
        },
    };

    let scheduler = match tokio::fs::OpenOptions::new()
        .read(true)
        .open(SAVE.get().unwrap().join("tasks.json"))
        .await
    {
        tokio::io::Result::Ok(file) => serde_json::from_reader(file.into_std().await)?,
        tokio::io::Result::Err(e) => match e.kind() {
            ErrorKind::NotFound => Default::default(),
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

    let saved = SavedState {
        turtles,
        world,
        depots,
    };

    let mut live = LiveState::from_save(saved, scheduler);

    for turtle in live.turtles.iter() {
        live.tasks.add_turtle(&TurtleCommander::new(turtle.read().await.name,&live).await.unwrap())
    }

    Ok(live)
}

#[derive(Serialize, Deserialize)]
struct SavedState {
    turtles: Vec<turtle::Turtle>,
    world: RTree<Block>,
    depots: Vec<Position>,
    //chunkloaders: unimplemented!(),
}

struct LiveState {
    turtles: Vec<Arc<RwLock<turtle::Turtle>>>,
    tasks: Scheduler,
    world: blocks::World,
    depots: Depots,
    started: Instant,
}

impl LiveState {
    async fn save(&self) -> SavedState {
        let mut turtles = Vec::new();
        for turtle in self.turtles.iter() {
            turtles.push(turtle.read().await.info());
        };
        let depots = self.depots.clone().to_vec().await;
        SavedState { turtles, world: self.world.tree().await, depots }
    }

    fn from_save(save: SavedState, scheduler: Scheduler) -> Self {
        let mut turtles = Vec::new();
        for turtle in save.turtles.into_iter() {
            let (tx, rx) = mpsc::channel(1);
            turtles.push(Turtle::with_channel(turtle.name.to_num(), turtle.position, turtle.fuel, turtle.fuel_limit, tx, rx));
        };
        let depots = Depots::from_vec(save.depots);
            
        Self { turtles: turtles.into_iter().map(|t| Arc::new(RwLock::new(t))).collect(), tasks: scheduler, world: World::from_tree(save.world),
            depots,
            started: Instant::now(),
        }
    }

    async fn get_turtle(&self, name: u32) -> Option<TurtleCommander> {
        TurtleCommander::new(Name::from_num(name), self).await
    }
    
}

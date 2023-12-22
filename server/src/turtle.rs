
use crate::SharedControl;
use crate::blocks::Block;
use crate::blocks::Direction;
use crate::blocks::Position;
use crate::blocks::Vec3;
use crate::blocks::World;
use crate::blocks::nearest;
use crate::mine::TurtleMineJob;
use crate::paths;
use crate::paths::route_facing;

use anyhow::Ok;

use anyhow;
use anyhow::Context;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::oneshot::channel;
use tokio::time::timeout;

use super::LiveState;

use std::collections::VecDeque;
use std::future::Ready;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;


use super::names::Name;

use serde::Deserialize;
use serde::Serialize;

use super::paths::route;

/// Time (ms) to wait for a command before letting a turtle go idle
const COMMAND_TIMEOUT:  u64 = 0o372;
/// Time (s) between turtle polls when idle
const IDLE_TIME: u32 = 3;

#[derive(Serialize, Deserialize)]
pub(crate) struct Turtle {
    pub(crate) name: Name,
    pub(crate) fuel: usize,
    pub(crate) fuel_limit: usize,
    /// movement vector of last given command
    pub(crate) queued_movement: Vec3,
    pub(crate) position: Position,
    pub(crate) goal: Option<Iota>,
    pub(crate) pending_update: bool,
    #[serde(skip)]
    callback: Option<oneshot::Sender<TurtleInfo>>,
    #[serde(skip)]
    sender: Option<Arc<Sender>>,
    #[serde(skip)]
    receiver: Option<Receiver>,
}

#[derive(Debug)]
pub struct TurtleInfo {
    pub name: Name,
    pub pos: Position,
    pub fuel: usize,
    /// Block name
    pub ahead: String,
    pub above: String,
    pub below: String,
    pub ret: TurtleCommandResponse,
}

impl TurtleInfo {
    fn from_update(update: TurtleUpdate, name: Name, pos: Position) -> Self {
        Self { name, pos,
        fuel: update.fuel, ahead: update.ahead, above: update.above, below: update.below, ret: update.ret }
    }
}

pub type Sender = mpsc::Sender<(TurtleCommand, oneshot::Sender<TurtleInfo>)>;
pub type Receiver = mpsc::Receiver<(TurtleCommand, oneshot::Sender<TurtleInfo>)>;

impl Default for Turtle {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self { 
            name: Name::from_num(0),
            fuel: Default::default(),
            fuel_limit: Default::default(),
            queued_movement: Default::default(),
            position: Position::new(Vec3::zeros(), Direction::North),
            goal: None,
            pending_update: Default::default(),
            callback: None,
            sender: Some(Arc::new(sender)),
            receiver: Some(receiver),
        }
    }
}

impl Turtle {
    pub(crate) fn new(id: u32, position: Position, fuel: usize, fuel_limit: usize) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            fuel_limit,
            queued_movement: Vec3::new(0, 0, 0),
            position,
            pending_update: true,
            ..Default::default()

        }
    }

    /// Similar turtle for serialization
    pub fn info(&self) -> Self {
        Self {
            name: self.name,
            fuel: self.fuel,
            fuel_limit: self.fuel_limit,
            position: self.position,
            pending_update: self.pending_update,
            queued_movement: self.queued_movement,
            ..Default::default()
        }
    }

    pub fn with_channel(id: u32, position: Position, fuel: usize, fuel_limit: usize, sender: Sender, receiver: Receiver) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            fuel_limit,
            queued_movement: Vec3::new(0, 0, 0),
            position,
            pending_update: true,
            sender: Some(Arc::new(sender)),
            receiver: Some(receiver),
            ..Default::default()

        }
    }
}

#[derive(Clone)]
pub struct TurtleCommander {
    sender: Arc<Sender>,
    world: World,
    // everything below is best-effort
    // TODO: make not bad
    pos: Arc<RwLock<Position>>,
    fuel: Arc<AtomicUsize>,
    max_fuel: Arc<AtomicUsize>,
    
}

impl TurtleCommander {
    pub async fn new(turtle: Name, state: &LiveState) -> Option<TurtleCommander> {
        let turtle = state.turtles.get(turtle.to_num() as usize)?.clone();
        let turtle = turtle.read().await;
        Some(TurtleCommander { 
            sender: turtle.sender.as_ref().unwrap().clone(),
            world: state.world.clone(),
            pos: Arc::new(RwLock::new(turtle.position)),
            fuel: Arc::new(AtomicUsize::new(turtle.fuel)),
            max_fuel: Arc::new(AtomicUsize::new(turtle.fuel_limit)),
        })
    }

    pub async fn execute(&self, command: TurtleCommand) -> TurtleInfo {
        let (send, recv) = oneshot::channel::<TurtleInfo>();

        self.sender.to_owned().send((command,send)).await.unwrap();

        let resp = recv.await.unwrap();
        let mut pos = self.pos.write().await;
        *pos = resp.pos;
        self.fuel.store(resp.fuel, std::sync::atomic::Ordering::SeqCst);
        resp
    }

    pub async fn pos(&self) -> Position {
        self.pos.read().await.clone()
    }

    pub async fn fuel(&self) -> usize {
        self.fuel.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub async fn fuel_limit(&self) -> usize {
        self.max_fuel.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn world(&self) -> World {
        self.world.clone()
    }

    pub async fn goto(&self, pos: Position) -> Option<()> {
        let mut recent = self.pos().await;
        let world = self.world.clone();
        loop {
            if recent == pos {
                break;
            }

            // easiest way to not eventually take over all memory
            let routing = timeout(Duration::from_secs(2), route(recent, pos, &world));
            let route = routing.await.ok()??;

            let steps: Vec<TurtleCommand> = route.iter().map_windows(|[from,to]| from.difference(**to).unwrap()).collect();

            'route: for (next_position, command) in route.into_iter().skip(1).zip(steps) {
                // reroute if the goal point is not empty before moving
                // valid routes will explicitly tell you to break ground

                if world.occupied(next_position.pos).await {
                    if world.garbage(next_position.pos).await {
                        match recent.dig(next_position.pos) {
                            Some(command) => self.execute(command).await,
                            None => break 'route,
                        };
                    } else {
                        break 'route;
                    }
                }

                let state = self.execute(command.clone()).await;

                if let TurtleCommandResponse::Failure =  state.ret {
                    if let TurtleCommand::Backward(_) = command {
                        // turn around if you bump your rear on something
                        self.execute(TurtleCommand::Left).await;
                        recent = self.execute(TurtleCommand::Left).await.pos;
                    }
                    break 'route;
                }

                recent = state.pos;
            }
        }
        Some(())
    }

    pub async fn goto_adjacent(&self, pos: Vec3) -> Option<Position> {
        let mut recent = self.pos().await;
        let world = self.world.clone();
        loop {
            
            if pos == recent.dir.unit() + recent.pos 
                || pos == recent.pos + Vec3::y()
                || pos == recent.pos - Vec3::y()
            {
                break;
            }

            let routing = timeout(Duration::from_secs(1), route_facing(recent, pos, &world));
            let route = routing.await.ok()??;

            let steps: Vec<TurtleCommand> = route.iter().map_windows(|[from,to]| from.difference(**to).unwrap()).collect();

            'route: for (next_position, command) in route.into_iter().skip(1).zip(steps) {
                if world.occupied(next_position.pos).await {
                    if world.garbage(next_position.pos).await {
                        let command = recent.dig(next_position.pos);
                        match command {
                            Some(command) => self.execute(command).await,
                            None => break 'route,
                        };
                    } else {
                        break 'route;
                    }
                }

                let state = self.execute(command.clone()).await;

                if let TurtleCommandResponse::Failure =  state.ret {
                    if let TurtleCommand::Backward(_) = command {
                        // turn around if you bump your rear on something
                        self.execute(TurtleCommand::Left).await;
                        recent = self.execute(TurtleCommand::Left).await.pos;
                    }
                    break 'route;
                }

                recent = state.pos;
            }
        }
        Some(recent)
    }
}


pub(crate) async fn process_turtle_update(
    id: u32,
    state: &mut LiveState,
    update: TurtleUpdate,
) -> anyhow::Result<TurtleCommand> {
    let mut  turtle = state
        .turtles
        .get(id as usize)
        .context("nonexisting turtle")?.write().await;
    let world = &mut state.world;

    if turtle.pending_update {
        turtle.pending_update = false;
        return Ok(TurtleCommand::Update);
    }

    if turtle.fuel > update.fuel {
        let diff = turtle.fuel - update.fuel;

        let delta = turtle.queued_movement * diff as i32;

        turtle.position.pos += delta;
        turtle.queued_movement = Vec3::zeros();
    }
    turtle.fuel = update.fuel;

    let above = Block {
        name: update.above.clone(),
        pos: turtle.position.pos + Vec3::y(),
    };
    world.set(above.clone()).await;

    let ahead = Block {
        name: update.ahead.clone(),
        pos: turtle.position.pos + turtle.position.dir.unit(),
    };
    world.set(ahead.clone()).await;

    let below = Block {
        name: update.below.clone(),
        pos: turtle.position.pos - Vec3::y(),
    };
    world.set(below.clone()).await;

    let info = TurtleInfo::from_update(update, turtle.name.clone(), turtle.position.clone());

    if let Some(send) = turtle.callback.take() {
        send.send(info).unwrap();
    }

    if let Some(recv) = turtle.receiver.as_mut() {
        let next = timeout(Duration::from_millis(COMMAND_TIMEOUT), recv.recv());
        if let Some((cmd, ret)) = next.await.ok().flatten() {
            turtle.callback = Some(ret);

            match cmd {
                TurtleCommand::Left => turtle.position.dir = turtle.position.dir.left(),
                TurtleCommand::Right => turtle.position.dir = turtle.position.dir.right(),
                _ => {}
            }
            turtle.queued_movement = cmd.unit(turtle.position.dir);
            println!("{}: {cmd:?}", turtle.name.to_str());
            return Ok(cmd);
        }
    }

    println!("{} idle, connected", turtle.name.to_str());
    Ok(TurtleCommand::Wait(IDLE_TIME))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) enum TurtleCommand {
    Wait(u32),
    Forward(u32),
    Backward(u32),
    Up(u32),
    Down(u32),
    Left,
    Right,
    Dig,
    DigUp,
    DigDown,
    PlaceUp,
    Place,
    PlaceDown,
    /// Count
    DropFront(u32),
    DropUp(u32),
    DropDown(u32),
    SuckFront(u32),
    SuckUp(u32),
    SuckDown(u32),
    Select(u32),
    /// Slot in inventory
    ItemInfo(u32),
    Update,
    Poweroff,
    Refuel,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct InventorySlot {
    pub(crate) name: String,
    pub(crate) count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) enum TurtleCommandResponse {
    None,
    Success,
    Failure,
    Item(InventorySlot),
    Inventory(Vec<InventorySlot>),
}

impl TurtleCommand {
    pub(crate) fn delta(&self, direction: Direction) -> Vec3 {
        let dir = direction.unit();
        match self {
            TurtleCommand::Forward(count) => dir * *count as i32,
            TurtleCommand::Backward(count) => -dir * *count as i32,
            TurtleCommand::Up(count) => Vec3::y() * *count as i32,
            TurtleCommand::Down(count) => -Vec3::y() * *count as i32,
            _ => Vec3::zeros(),
        }
    }

    pub(crate) fn unit(&self, direction: Direction) -> Vec3 {
        let dir = direction.unit();
        match self {
            TurtleCommand::Forward(_) => dir,
            TurtleCommand::Backward(_) => -dir,
            TurtleCommand::Up(_) => Vec3::y(),
            TurtleCommand::Down(_) => -Vec3::y(),
            _ => Vec3::zeros(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct TurtleUpdate {
    pub(crate) fuel: usize,
    /// Block name
    pub(crate) ahead: String,
    pub(crate) above: String,
    pub(crate) below: String,
    pub(crate) ret: TurtleCommandResponse,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TurtleRegister {
    pub(crate) fuel: usize,
    pub(crate) fuellimit: usize,
    pub(crate) position: Vec3,
    pub(crate) facing: Direction,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TurtleResponse {
    pub(crate) name: String,
    pub(crate) id: u32,
    pub(crate) command: TurtleCommand,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum Iota {
    End,
    Goto(Position),
    Mine(Vec3),
    Execute(TurtleCommand),
}

pub trait TurtleTask: erased_serde::Serialize {
    fn handle_block(&mut self, _: Block) { }
    fn next(&mut self, turtle: &Turtle) -> Iota
    { Iota::End }
}

erased_serde::serialize_trait_object!(TurtleTask);

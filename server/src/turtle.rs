
use crate::blocks::Block;
use crate::blocks::Direction;
use crate::blocks::Position;
use crate::blocks::Vec3;
use crate::blocks::World;
use crate::blocks::nearest;
use crate::mine::TurtleMineJob;

use anyhow::Ok;

use anyhow;
use anyhow::Context;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::oneshot::channel;

use super::LiveState;

use std::collections::VecDeque;
use std::future::Ready;
use std::sync::Arc;


use super::names::Name;

use serde::Deserialize;
use serde::Serialize;

use super::paths::route;

#[derive(Serialize, Deserialize)]
pub(crate) struct Turtle {
    pub(crate) name: Name,
    pub(crate) fuel: usize,
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
    pub(crate) fn new(id: u32, position: Position, fuel: usize) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
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
            position: self.position,
            pending_update: self.pending_update,
            queued_movement: self.queued_movement,
            ..Default::default()
        }
    }

    pub fn with_channel(id: u32, position: Position, fuel: usize, sender: Sender, receiver: Receiver) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            queued_movement: Vec3::new(0, 0, 0),
            position,
            pending_update: true,
            sender: Some(Arc::new(sender)),
            receiver: Some(receiver),
            ..Default::default()

        }
    }

    pub fn cmd(&self) -> TurtleCommander {
        TurtleCommander { sender: self.sender.as_ref().unwrap().clone() }
    }
        
}

#[derive(Clone)]
pub struct TurtleCommander {
    sender: Arc<Sender>,
}

impl TurtleCommander {
    pub async fn execute(&self, command: TurtleCommand) -> TurtleInfo {
        let (send, recv) = oneshot::channel::<TurtleInfo>();

        self.sender.to_owned().send((command,send)).await.unwrap();

        recv.await.unwrap()
    }
}

pub async fn goto(cmd: TurtleCommander, recent: TurtleInfo, pos: Position, world: World) -> Option<()> {
    let mut recent = recent.pos;
    loop {
        if recent == pos {
            break;
        }

        let route = route(recent, pos, &world).await?;

        let steps: Vec<TurtleCommand> = route.iter().map_windows(|[from,to]| from.difference(**to).unwrap()).collect();

        'route: for (next_position, command) in route.into_iter().skip(1).zip(steps) {
            // reroute if the goal point is not empty before moving
            // valid routes will explicitly tell you to break ground

            if world.occupied(next_position.pos).await {
                break 'route;
            }

            let state = cmd.execute(command).await;
            recent = state.pos;
        }
    }
    Some(())
}

pub(crate) async fn process_turtle_update(
    id: u32,
    state: &mut LiveState,
    update: TurtleUpdate,
) -> anyhow::Result<TurtleCommand> {
    let turtle = state
        .turtles
        .get_mut(id as usize)
        .context("nonexisting turtle")?;
    let tasks = state
        .tasks
        .get_mut(id as usize)
        .context("state gone?????").unwrap();
    let world = &mut state.world;

    if turtle.pending_update {
        turtle.pending_update = false;
        return Ok(TurtleCommand::Update);
    }

    if turtle.fuel != update.fuel {
        turtle.fuel = update.fuel;

        turtle.position.pos += turtle.queued_movement;
        turtle.queued_movement = Vec3::zeros();
    }

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
        if let Some((cmd, ret)) = recv.try_recv().ok() {
            turtle.callback = Some(ret);

            match cmd {
                TurtleCommand::Left => turtle.position.dir = turtle.position.dir.left(),
                TurtleCommand::Right => turtle.position.dir = turtle.position.dir.right(),
                _ => {}
            }
            return Ok(cmd);
        }
    }

    Ok(TurtleCommand::Wait(3))
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
    Select(u32),
    /// Slot in inventory
    ItemInfo(u32),
    Update,
    Poweroff,
    Refuel,
    Dump,
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

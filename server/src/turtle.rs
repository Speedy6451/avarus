
use crate::blocks::Block;
use crate::blocks::Direction;
use crate::blocks::Position;
use crate::blocks::Vec3;
use crate::blocks::nearest;
use crate::mine::TurtleMineJob;

use anyhow::Ok;

use anyhow;
use anyhow::Context;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use super::ControlState;

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
        Self { 
            name: Name::from_num(0),
            fuel: Default::default(),
            queued_movement: Default::default(),
            position: (Vec3::zeros(), Direction::North),
            goal: None,
            pending_update: Default::default(),
            callback: None,
            sender: None,
            receiver: None,
        }
    }
}

impl Turtle {
    pub(crate) fn new(id: u32, position: Vec3, facing: Direction, fuel: usize) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            queued_movement: Vec3::new(0, 0, 0),
            position: (position, facing),
            pending_update: true,
            ..Default::default()

        }
    }

    pub fn with_channel(id: u32, position: Vec3, facing: Direction, fuel: usize, sender: Sender, receiver: Receiver) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            queued_movement: Vec3::new(0, 0, 0),
            position: (position, facing),
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

async fn goto(cmd: &TurtleCommander, recent: TurtleInfo, pos: Position, world: &rstar::RTree<Block>) -> Option<()> {
    let mut recent = recent.pos;
    loop {
        if recent == pos {
            break;
        }

        let route = route(recent, pos, world)?;

        let steps: Vec<TurtleCommand> = route.iter().map_windows(|[from,to]| difference(**from,**to).unwrap()).collect();

        'route: for (next_position, command) in route.into_iter().skip(1).zip(steps) {
            // reroute if the goal point is not empty before moving
            // valid routes will explicitly tell you to break ground

            if world.locate_at_point(&next_position.0.into()).unwrap().name != "minecraft:air" {
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
    state: &mut ControlState,
    update: TurtleUpdate,
) -> anyhow::Result<TurtleCommand> {
    let turtle = state
        .saved.turtles
        .get_mut(id as usize)
        .context("nonexisting turtle")?;
    let tasks = state
        .saved.tasks
        .get_mut(id as usize)
        .context("state gone?????").unwrap();
    let world = &mut state.saved.world;

    if turtle.pending_update {
        turtle.pending_update = false;
        return Ok(TurtleCommand::Update);
    }

    if turtle.fuel != update.fuel {
        turtle.fuel = update.fuel;

        turtle.position.0 += turtle.queued_movement;
        turtle.queued_movement = Vec3::zeros();
    }

    let above = Block {
        name: update.above.clone(),
        pos: turtle.position.0 + Vec3::y(),
    };
    world.remove_at_point(&above.pos.into());
    world.insert(above.clone());

    let ahead = Block {
        name: update.ahead.clone(),
        pos: turtle.position.0 + turtle.position.1.clone().unit(),
    };
    world.remove_at_point(&ahead.pos.into());
    world.insert(ahead.clone());

    let below = Block {
        name: update.below.clone(),
        pos: turtle.position.0 - Vec3::y(),
    };
    world.remove_at_point(&below.pos.into());
    world.insert(below.clone());

    let info = TurtleInfo::from_update(update, turtle.name.clone(), turtle.position.clone());

    if let Some(send) = turtle.callback.take() {
        send.send(info).unwrap();
    }

    if let Some(recv) = turtle.receiver.as_mut() {
        if let Some((cmd, ret)) = recv.try_recv().ok() {
            turtle.callback = Some(ret);

            return Ok(cmd);
        }
    }

    if let Some(goal) = turtle.goal.take().or_else(|| tasks.front_mut().map(|t| t.next(&turtle))) {
         let command = match goal {
            Iota::End => {
                tasks.pop_front();
                TurtleCommand::Wait(0) // TODO: fix
            },
            Iota::Goto(pos) => {
                println!("gogto: {:?}", pos);
                pathstep(turtle, world, pos).unwrap()
            },
            Iota::Mine(pos) => {
                let pos = nearest(turtle.position.0, pos);
                
                if pos == turtle.position {
                    TurtleCommand::Dig
                } else {
                    pathstep(turtle, world, pos).unwrap()
                }
            },
            Iota::Execute(cmd) => {
                cmd
            },
        };

         println!("Order: {:?}", command);
        return Ok(command);
    };

    Ok(TurtleCommand::Wait(3))
}

fn pathstep(turtle: &mut Turtle, world: &mut rstar::RTree<Block>, goal: Position) -> Option<TurtleCommand> {
    // TODO: memoize this whenever we aren't digging
    let route = route(turtle.position, goal, world)?;
    let mut next_move = difference(route[0], route[1])?;
    if world
        .locate_at_point(&route[1].0.into())
        .is_some_and(|b| b.name != "minecraft:air")
    {
        next_move = match next_move {
            TurtleCommand::Up(_) => TurtleCommand::DigUp,
            TurtleCommand::Down(_) => TurtleCommand::DigDown,
            TurtleCommand::Forward(_) => TurtleCommand::Dig,
            _ => next_move,
        }
    }
    turtle.queued_movement = next_move.delta(turtle.position.1);
    match next_move {
        TurtleCommand::Left => turtle.position.1 = turtle.position.1.left(),
        TurtleCommand::Right => turtle.position.1 = turtle.position.1.right(),
        _ => {}
    }
    return Some(next_move);
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

/// Get a turtle command to map two adjacent positions
fn difference(from: Position, to: Position) -> Option<TurtleCommand> {
    use TurtleCommand::*;

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

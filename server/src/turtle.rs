
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

use super::ControlState;

use std::collections::VecDeque;
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
    pub(crate) tasks: VecDeque<RwLock<Arc<dyn TurtleTask + Send + Sync>>>,
}

impl Default for Turtle {
    fn default() -> Self {
        Self { 
            name: Name::from_num(0),
            fuel: Default::default(),
            queued_movement: Default::default(),
            position: (Vec3::zeros(), Direction::North),
            goal: Default::default(),
            pending_update: Default::default(),
            tasks: VecDeque::new(),
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
            goal: None,
            pending_update: true,
            tasks: VecDeque::new(),
        }
    }

    pub fn add_task(&mut self, task: impl TurtleTask + Send + Sync) {
        self.tasks.push_back(Arc::new(task));
    }
}

pub(crate) fn process_turtle_update(
    id: u32,
    state: &mut ControlState,
    update: TurtleUpdate,
) -> anyhow::Result<TurtleCommand> {
    let turtle = state
        .turtles
        .get_mut(id as usize)
        .context("nonexisting turtle")?;
    let world = &mut state.world;

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
        name: update.above,
        pos: turtle.position.0 + Vec3::y(),
    };
    world.remove_at_point(&above.pos.into());
    world.insert(above);

    let ahead = Block {
        name: update.ahead,
        pos: turtle.position.0 + turtle.position.1.clone().unit(),
    };
    world.remove_at_point(&ahead.pos.into());
    world.insert(ahead);

    let below = Block {
        name: update.below,
        pos: turtle.position.0 - Vec3::y(),
    };
    world.remove_at_point(&below.pos.into());
    world.insert(below);

    if let Some(task) = turtle.tasks.front() {
        task.handle_block(above);
        task.handle_block(below);
        task.handle_block(ahead);
    }

    if let Some(goal) = turtle.tasks.front().map(|t| t.next(&turtle)) {
         let command = match goal {
            Iota::End => {
                turtle.tasks.pop_front();
                TurtleCommand::Wait(0) // TODO: fix
            },
            Iota::Goto(pos) => {
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

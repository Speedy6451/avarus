use super::TurtleMineJob;

use super::difference;

use crate::blocks::Block;

use anyhow::Ok;

use anyhow;
use anyhow::Context;

use super::ControlState;

use super::Direction;

use std::collections::VecDeque;

use super::Position;

use super::Vec3;

use super::names::Name;

use serde::Serialize;
use serde::Deserialize;

use super::paths::route;

#[derive(Serialize, Deserialize)]
pub(crate) struct Turtle {
    pub(crate) name: Name,
    pub(crate) fuel: usize,
    /// movement vector of last given command
    pub(crate) queued_movement: Vec3,
    pub(crate) position: Position,
    pub(crate) goal: Option<Position>,
    pub(crate) pending_update: bool,
    pub(crate) moves: VecDeque<TurtleCommand>,
}

impl Turtle {
    pub(crate) fn new(id: u32, position: Vec3, facing: Direction, fuel: usize) -> Self {
        Self { name: Name::from_num(id), fuel, queued_movement: Vec3::new(0, 0, 0), position: (position, facing), goal: None, pending_update: true, moves: VecDeque::new() }

    }
}

pub(crate) fn process_turtle_update(
    id: u32,
    state: &mut ControlState,
    update: TurtleUpdate,
    ) -> anyhow::Result<TurtleCommand> {
    let turtle = state.turtles.get_mut(id as usize).context("nonexisting turtle")?;
    let world = &mut state.world;

    if turtle.pending_update {
        turtle.pending_update = false;
        return Ok(TurtleCommand::Update);
    }

    println!("above: {}, below: {}, ahead: {}", update.above, update.below, update.ahead);
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

    if turtle.goal.is_some_and(|g| g == turtle.position) {
        turtle.goal = None;
    }

    if let Some(goal) = turtle.goal {
        // TODO: memoize this whenever we aren't digging
        let route = route(turtle.position, goal, world).unwrap();
        println!("route: {:?}", route);
        let mut next_move = difference(route[0], route[1]).unwrap();
        if world.locate_at_point(&route[1].0.into()).is_some_and(|b| b.name != "minecraft:air") {
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
            _ => {},
        }
        return Ok(next_move);
    }

    Ok(TurtleCommand::Wait(3))
}

#[derive(Serialize, Deserialize)]
pub(crate) enum TurtleTask {
    Mining(TurtleMineJob),
    Idle,
}

#[derive(Serialize, Deserialize, Clone)]
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
